use std::{
    error::Error,
    fmt::{self, Display},
    mem::{size_of, MaybeUninit},
    path::Path,
    sync::Arc,
};

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

#[cfg(target_os = "wasi")]
use std::os::wasi::ffi::OsStrExt;

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

use crate::{
    ffi::{
        DependenciesFFI, DynDependencies, DynSource, ImporterFFI, ImporterImportFn, ImporterOpaque,
        SourcesFFI, ANY_BUF_LEN_LIMIT, BUFFER_IS_TOO_SMALL, MAX_EXTENSION_COUNT, MAX_FFI_NAME_LEN,
        MAX_FORMATS_COUNT, OTHER_ERROR, REQUIRE_DEPENDENCIES, REQUIRE_SOURCES, SUCCESS,
    },
    importer::Importer,
    version, Dependencies, Dependency, ImportError, Sources, MAGIC,
};

const RESULT_BUF_LEN_START: usize = 1024;

type MagicType = u32;
const MAGIC_NAME: &'static str = "ARGOSY_DYLIB_MAGIC";

type VersionFnType = unsafe extern "C" fn() -> u32;
const VERSION_FN_NAME: &'static str = "argosy_importer_ffi_version_minor";

type ExportImportersFnType = unsafe extern "C" fn(buffer: *mut ImporterFFI, count: u32) -> u32;
const EXPORT_IMPORTERS_FN_NAME: &'static str = "argosy_export_importers";

pub struct DylibImporter {
    _path: Arc<Path>,
    _library: Arc<libloading::Library>,
    importer: *const ImporterOpaque,
    import: ImporterImportFn,
    name: [u8; MAX_FFI_NAME_LEN],
    formats: [Box<str>; MAX_FORMATS_COUNT],
    target: [u8; MAX_FFI_NAME_LEN],
    extensions: [Box<str>; MAX_EXTENSION_COUNT],
}

/// Exporting non thread-safe importers breaks the contract of the FFI.
/// The potential unsoundness is covered by `load_dylib_importers` unsafety.
/// There is no way to guarantee that dynamic library will uphold the contract,
/// making `load_dylib_importers` inevitably unsound.
unsafe impl Send for DylibImporter {}
unsafe impl Sync for DylibImporter {}

impl DylibImporter {
    fn new(importer: ImporterFFI, path: Arc<Path>, library: Arc<libloading::Library>) -> Self {
        DylibImporter {
            _path: path,
            _library: library,
            importer: importer.importer,
            import: importer.import,
            name: importer.name,
            formats: importer
                .formats
                .map(|format| unsafe { std::str::from_utf8_unchecked(&format).into() }),
            target: importer.target,
            extensions: importer
                .extensions
                .map(|extension| unsafe { std::str::from_utf8_unchecked(&extension).into() }),
        }
    }
}

impl Importer for DylibImporter {
    fn name(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(&self.name) }
    }

    fn formats(&self) -> &[&str] {
        unsafe {
            std::slice::from_raw_parts(self.formats.as_ptr() as *const &str, self.formats.len())
        }
    }

    fn target(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(&self.target) }
    }

    fn extensions(&self) -> &[&str] {
        unsafe {
            std::slice::from_raw_parts(
                self.extensions.as_ptr() as *const &str,
                self.extensions.len(),
            )
        }
    }

    fn import(
        &self,
        source: &Path,
        output: &Path,
        sources: &mut dyn Sources,
        dependencies: &mut dyn Dependencies,
    ) -> Result<(), ImportError> {
        let os_str = source.as_os_str();

        #[cfg(any(unix, target_os = "wasi"))]
        let source: &[u8] = os_str.as_bytes();

        #[cfg(windows)]
        let os_str_wide = os_str.encode_wide().collect::<Vec<u16>>();

        #[cfg(windows)]
        let source: &[u16] = &*os_str_wide;

        let os_str = output.as_os_str();

        #[cfg(any(unix, target_os = "wasi"))]
        let output: &[u8] = os_str.as_bytes();

        #[cfg(windows)]
        let os_str_wide = os_str.encode_wide().collect::<Vec<u16>>();

        #[cfg(windows)]
        let output: &[u16] = &*os_str_wide;

        let mut sources = DynSource::new(sources);
        let sources = SourcesFFI::new(&mut sources);

        let mut dependencies = DynDependencies::new(dependencies);
        let dependencies = DependenciesFFI::new(&mut dependencies);

        let mut result_buf = vec![0; RESULT_BUF_LEN_START];
        let mut result_len = result_buf.len() as u32;

        let result = loop {
            let result = unsafe {
                (self.import)(
                    self.importer,
                    source.as_ptr(),
                    source.len() as u32,
                    output.as_ptr(),
                    output.len() as u32,
                    sources.opaque,
                    sources.get,
                    dependencies.opaque,
                    dependencies.get,
                    result_buf.as_mut_ptr(),
                    &mut result_len,
                )
            };

            if result == BUFFER_IS_TOO_SMALL {
                if result_len > ANY_BUF_LEN_LIMIT as u32 {
                    return Err(ImportError::Other {
                        reason: format!(
                            "Result does not fit into limit '{}', '{}' required",
                            ANY_BUF_LEN_LIMIT, result_len
                        ),
                    });
                }

                result_buf.resize(result_len as usize, 0);
            }
            break result;
        };

        match result {
            SUCCESS => Ok(()),
            REQUIRE_SOURCES => unsafe {
                let mut u32buf = [0; size_of::<u32>()];
                std::ptr::copy_nonoverlapping(
                    result_buf[..size_of::<u32>()].as_ptr(),
                    u32buf.as_mut_ptr(),
                    size_of::<u32>(),
                );
                let count = u32::from_le_bytes(u32buf);

                let mut offset = size_of::<u32>();

                let mut sources = Vec::new();
                for _ in 0..count {
                    std::ptr::copy_nonoverlapping(
                        result_buf[offset..][..size_of::<u32>()].as_ptr(),
                        u32buf.as_mut_ptr(),
                        size_of::<u32>(),
                    );
                    offset += size_of::<u32>();
                    let len = u32::from_le_bytes(u32buf);
                    let mut source = vec![0; len as usize];
                    std::ptr::copy_nonoverlapping(
                        result_buf[offset..][..len as usize].as_ptr(),
                        source.as_mut_ptr(),
                        len as usize,
                    );
                    offset += len as usize;
                    match String::from_utf8(source) {
                            Ok(source) => sources.push(source),
                            Err(_) => return Err(ImportError::Other {
                                reason: "`Importer::import` requires sources, but one of the sources is not UTF-8"
                                    .to_owned(),
                            }),
                        }
                }

                Err(ImportError::RequireSources { sources })
            },
            REQUIRE_DEPENDENCIES => unsafe {
                let mut u32buf = [0; size_of::<u32>()];
                std::ptr::copy_nonoverlapping(
                    result_buf[..size_of::<u32>()].as_ptr(),
                    u32buf.as_mut_ptr(),
                    size_of::<u32>(),
                );
                let count = u32::from_le_bytes(u32buf);
                let mut offset = size_of::<u32>();

                let mut dependencies = Vec::new();
                for _ in 0..count {
                    let mut decode_string = || {
                        std::ptr::copy_nonoverlapping(
                            result_buf[offset..][..size_of::<u32>()].as_ptr(),
                            u32buf.as_mut_ptr(),
                            size_of::<u32>(),
                        );
                        offset += size_of::<u32>();
                        let len = u32::from_le_bytes(u32buf);

                        let mut string = vec![0; len as usize];
                        std::ptr::copy_nonoverlapping(
                            result_buf[offset..][..len as usize].as_ptr(),
                            string.as_mut_ptr(),
                            len as usize,
                        );
                        offset += len as usize;

                        match String::from_utf8(string) {
                                Ok(string) => Ok(string),
                                Err(_) => return Err(ImportError::Other { reason: "`Importer::import` requires dependencies, but one of the strings is not UTF-8".to_owned() }),
                            }
                    };

                    let source = decode_string()?;
                    let target = decode_string()?;

                    dependencies.push(Dependency { source, target });
                }

                Err(ImportError::RequireDependencies { dependencies })
            },
            OTHER_ERROR => {
                debug_assert!(result_len <= result_buf.len() as u32);

                let error = &result_buf[..result_len as usize];
                let error_lossy = String::from_utf8_lossy(error);

                Err(ImportError::Other {
                    reason: error_lossy.into_owned(),
                })
            }
            _ => Err(ImportError::Other {
                reason: format!(
                    "Unexpected return code from `Importer::import` FFI: {}",
                    result
                ),
            }),
        }
    }
}

#[derive(Debug)]
pub enum LoadingError {
    LibLoading(libloading::Error),
    FailedToOpenLibrary,
    MagicSymbolNotFound,
    MagicValueMismatch,
    VersionSymbolNotFound,
    VersionMismatch,
    ExportImportersSymbolNotFound,
}

impl Display for LoadingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadingError::LibLoading(err) => write!(f, "libloading error: {}", err),
            LoadingError::FailedToOpenLibrary => write!(f, "Failed to open library"),
            LoadingError::MagicSymbolNotFound => {
                write!(f, "'ARGOSY_DYLIB_MAGIC' symbol not found")
            }
            LoadingError::MagicValueMismatch => {
                write!(f, "'ARGOSY_DYLIB_MAGIC' value mismatch")
            }
            LoadingError::VersionSymbolNotFound => {
                write!(f, "'argosy_importer_ffi_version_minor' symbol not found")
            }
            LoadingError::VersionMismatch => write!(f, "Version mismatch"),
            LoadingError::ExportImportersSymbolNotFound => {
                write!(f, "'argosy_export_importers' symbol not found")
            }
        }
    }
}

impl Error for LoadingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            LoadingError::LibLoading(err) => Some(err),
            _ => None,
        }
    }
}

/// Load importers from dynamic library at specified path.
pub unsafe fn load_importers(
    lib_path: &Path,
) -> Result<impl Iterator<Item = DylibImporter>, LoadingError> {
    tracing::info!("Loading importers from '{}'", lib_path.display());

    let lib = libloading::Library::new(lib_path).map_err(|_| LoadingError::FailedToOpenLibrary)?;

    // First check the magic value. It must be both present and equal the constant.
    let magic = lib
        .get::<*const MagicType>(MAGIC_NAME.as_bytes())
        .map_err(|_| LoadingError::MagicSymbolNotFound)?;

    if **magic != MAGIC {
        return Err(LoadingError::MagicValueMismatch);
    }

    // First check the magic value. It must be both present and equal the constant.
    let lib_ffi_version = lib
        .get::<VersionFnType>(VERSION_FN_NAME.as_bytes())
        .map_err(|_| LoadingError::VersionSymbolNotFound)?;

    let lib_ffi_version = lib_ffi_version();

    let ffi_version = version();

    if lib_ffi_version != ffi_version {
        return Err(LoadingError::VersionMismatch);
    }

    let export_importers = lib
        .get::<ExportImportersFnType>(EXPORT_IMPORTERS_FN_NAME.as_bytes())
        .map_err(|_| LoadingError::ExportImportersSymbolNotFound)?;

    let mut importers = Vec::new();
    importers.resize_with(64, MaybeUninit::uninit);

    loop {
        let count = export_importers(
            importers.as_mut_ptr() as *mut ImporterFFI,
            importers.len() as u32,
        );

        if count > importers.len() as u32 {
            importers.resize_with(count as usize, MaybeUninit::uninit);
            continue;
        }

        importers.truncate(count as usize);
        break;
    }

    let lib = Arc::new(lib);
    let lib_path: Arc<Path> = Arc::from(lib_path);

    Ok(importers.into_iter().map(move |importer| {
        let ffi: ImporterFFI = importer.assume_init();
        DylibImporter::new(ffi, lib_path.clone(), lib.clone())
    }))
}
