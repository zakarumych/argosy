use std::{mem::size_of, path::PathBuf};

#[cfg(unix)]
use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

#[cfg(target_os = "wasi")]
use std::{ffi::OsStr, os::wasi::ffi::OsStrExt};

#[cfg(windows)]
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
};

use asset_influx_id::AssetId;

use crate::{
    dependencies::Dependencies,
    importer::{ImportError, Importer},
    sources::Sources,
};

const PATH_BUF_LEN_START: usize = 1024;
pub const ANY_BUF_LEN_LIMIT: usize = 65536;

pub const REQUIRE_SOURCES: i32 = 2;
pub const REQUIRE_DEPENDENCIES: i32 = 1;
pub const SUCCESS: i32 = 0;
pub const NOT_FOUND: i32 = -1;
pub const NOT_UTF8: i32 = -2;
pub const BUFFER_IS_TOO_SMALL: i32 = -3;
pub const OTHER_ERROR: i32 = -6;

#[cfg(any(unix, target_os = "wasi"))]
type OsChar = u8;

#[cfg(windows)]
type OsChar = u16;

#[repr(transparent)]
pub struct DependenciesOpaque(u8);

pub type DependenciesGetFn = unsafe extern "C" fn(
    dependencies: *mut DependenciesOpaque,
    source_ptr: *const u8,
    source_len: u32,
    target_ptr: *const u8,
    target_len: u32,
    id_ptr: *mut u64,
) -> i32;

unsafe extern "C" fn dependencies_get_ffi(
    dependencies: *mut DependenciesOpaque,
    source_ptr: *const u8,
    source_len: u32,
    target_ptr: *const u8,
    target_len: u32,
    id_ptr: *mut u64,
) -> i32 {
    let source =
        match std::str::from_utf8(std::slice::from_raw_parts(source_ptr, source_len as usize)) {
            Ok(source) => source,
            Err(_) => return NOT_UTF8,
        };

    let target =
        match std::str::from_utf8(std::slice::from_raw_parts(target_ptr, target_len as usize)) {
            Ok(target) => target,
            Err(_) => return NOT_UTF8,
        };

    let f = dependencies as *mut DynDependencies;
    let f = &mut *f;

    match f.get(source, target) {
        Err(_) => return OTHER_ERROR,
        Ok(None) => return NOT_FOUND,
        Ok(Some(id)) => {
            std::ptr::write(id_ptr, id.value().get());
            return SUCCESS;
        }
    }
}

pub struct DependenciesFFI {
    pub opaque: *mut DependenciesOpaque,
    pub get: DependenciesGetFn,
}

pub struct DynDependencies<'a> {
    dependencies: &'a mut dyn Dependencies,
}

impl<'a> DynDependencies<'a> {
    pub fn new(dependencies: &'a mut dyn Dependencies) -> Self {
        DynDependencies { dependencies }
    }

    fn get(&mut self, source: &str, target: &str) -> Result<Option<AssetId>, String> {
        self.dependencies.get(source, target)
    }
}

impl DependenciesFFI {
    pub fn new(dependencies: &mut DynDependencies) -> Self {
        DependenciesFFI {
            opaque: dependencies as *mut DynDependencies as _,
            get: dependencies_get_ffi,
        }
    }
}

impl Dependencies for DependenciesFFI {
    fn get(&mut self, source: &str, target: &str) -> Result<Option<AssetId>, String> {
        let mut id = 0u64;
        let result = unsafe {
            (self.get)(
                self.opaque,
                source.as_ptr(),
                source.len() as u32,
                target.as_ptr(),
                target.len() as u32,
                &mut id,
            )
        };

        match result {
            SUCCESS => match AssetId::new(id) {
                None => Err(format!("Null AssetId returned from `Dependencies::get`")),
                Some(id) => Ok(Some(id)),
            },
            NOT_FOUND => Ok(None),
            NOT_UTF8 => Err(format!("Source is not UTF8 while stored in `str`")),

            _ => Err(format!(
                "Unexpected return code from `Sources::get` FFI: {}",
                result
            )),
        }
    }
}

#[repr(transparent)]
pub struct SourcesOpaque(u8);

pub type SourcesGetFn = unsafe extern "C" fn(
    sources: *mut SourcesOpaque,
    source_ptr: *const u8,
    source_len: u32,
    path_ptr: *mut OsChar,
    path_len: *mut u32,
) -> i32;

unsafe extern "C" fn sources_get_ffi<'a>(
    sources: *mut SourcesOpaque,
    source_ptr: *const u8,
    source_len: u32,
    path_ptr: *mut OsChar,
    path_len: *mut u32,
) -> i32 {
    let source =
        match std::str::from_utf8(std::slice::from_raw_parts(source_ptr, source_len as usize)) {
            Ok(source) => source,
            Err(_) => return NOT_UTF8,
        };

    let f = sources as *mut DynSource;
    let f = &mut *f;

    match f.get(source) {
        Err(_) => return OTHER_ERROR,
        Ok(None) => return NOT_FOUND,
        Ok(Some(path)) => {
            let os_str = path.as_os_str();

            #[cfg(any(unix, target_os = "wasi"))]
            let path: &[u8] = os_str.as_bytes();

            #[cfg(windows)]
            let os_str_wide = os_str.encode_wide().collect::<Vec<u16>>();

            #[cfg(windows)]
            let path: &[u16] = &*os_str_wide;

            if *path_len < path.len() as u32 {
                *path_len = path.len() as u32;
                return BUFFER_IS_TOO_SMALL;
            }

            std::ptr::copy_nonoverlapping(path.as_ptr(), path_ptr, path.len() as u32 as usize);
            *path_len = path.len() as u32;

            return SUCCESS;
        }
    }
}

pub struct SourcesFFI {
    pub opaque: *mut SourcesOpaque,
    pub get: SourcesGetFn,
}

pub struct DynSource<'a> {
    sources: &'a mut dyn Sources,
}

impl<'a> DynSource<'a> {
    pub fn new(sources: &'a mut dyn Sources) -> Self {
        DynSource { sources }
    }

    fn get(&mut self, source: &str) -> Result<Option<PathBuf>, String> {
        self.sources.get(source)
    }
}

impl SourcesFFI {
    pub fn new<'a>(sources: &mut DynSource) -> Self {
        SourcesFFI {
            opaque: sources as *const DynSource as _,
            get: sources_get_ffi,
        }
    }
}

impl Sources for SourcesFFI {
    fn get(&mut self, source: &str) -> Result<Option<PathBuf>, String> {
        let mut path_buf = vec![0; PATH_BUF_LEN_START];
        let mut path_len = path_buf.len() as u32;

        loop {
            let result = unsafe {
                (self.get)(
                    self.opaque,
                    source.as_ptr(),
                    source.len() as u32,
                    path_buf.as_mut_ptr(),
                    &mut path_len,
                )
            };

            if result == BUFFER_IS_TOO_SMALL {
                if path_len > ANY_BUF_LEN_LIMIT as u32 {
                    return Err(format!(
                        "Source path does not fit into limit '{}', '{}' required",
                        ANY_BUF_LEN_LIMIT, path_len
                    ));
                }

                path_buf.resize(path_len as usize, 0);
                continue;
            }

            return match result {
                SUCCESS => {
                    #[cfg(any(unix, target_os = "wasi"))]
                    let path = OsString::from_vec(path_buf).into();

                    #[cfg(windows)]
                    let path = OsString::from_wide(&path_buf).into();

                    Ok(Some(path))
                }
                NOT_FOUND => return Ok(None),
                NOT_UTF8 => Err(format!("Source is not UTF8 while stored in `str`")),
                _ => Err(format!(
                    "Unexpected return code from `Sources::get` FFI: {}",
                    result
                )),
            };
        }
    }
}

#[repr(transparent)]
pub struct ImporterOpaque(u8);

pub type ImporterImportFn = unsafe extern "C" fn(
    importer: *const ImporterOpaque,
    source_ptr: *const OsChar,
    source_len: u32,
    output_ptr: *const OsChar,
    output_len: u32,
    sources: *mut SourcesOpaque,
    sources_get: SourcesGetFn,
    dependencies: *mut DependenciesOpaque,
    dependencies_get: DependenciesGetFn,
    result_ptr: *mut u8,
    result_len: *mut u32,
) -> i32;

unsafe extern "C" fn importer_import_ffi<I>(
    importer: *const ImporterOpaque,
    source_ptr: *const OsChar,
    source_len: u32,
    output_ptr: *const OsChar,
    output_len: u32,
    sources: *mut SourcesOpaque,
    sources_get: SourcesGetFn,
    dependencies: *mut DependenciesOpaque,
    dependencies_get: DependenciesGetFn,
    result_ptr: *mut u8,
    result_len: *mut u32,
) -> i32
where
    I: Importer,
{
    let source = std::slice::from_raw_parts(source_ptr, source_len as usize);
    let output = std::slice::from_raw_parts(output_ptr, output_len as usize);

    #[cfg(any(unix, target_os = "wasi"))]
    let source = OsStr::from_bytes(source);
    #[cfg(any(unix, target_os = "wasi"))]
    let output = OsStr::from_bytes(output);

    #[cfg(windows)]
    let source = OsString::from_wide(source);
    #[cfg(windows)]
    let output = OsString::from_wide(output);

    let mut sources = SourcesFFI {
        opaque: sources,
        get: sources_get,
    };

    let mut dependencies = DependenciesFFI {
        opaque: dependencies,
        get: dependencies_get,
    };

    let importer = &*(importer as *const I);
    let result = importer.import(
        source.as_ref(),
        output.as_ref(),
        &mut sources,
        &mut dependencies,
    );

    match result {
        Ok(()) => SUCCESS,
        Err(ImportError::RequireSources { sources }) => {
            let len_required = sources
                .iter()
                .fold(0, |acc, p| acc + p.len() + size_of::<u32>())
                + size_of::<u32>();

            assert!(u32::try_from(len_required).is_ok());

            if *result_len < len_required as u32 {
                *result_len = len_required as u32;
                return BUFFER_IS_TOO_SMALL;
            }

            std::ptr::copy_nonoverlapping(
                (sources.len() as u32).to_le_bytes().as_ptr(),
                result_ptr,
                size_of::<u32>(),
            );

            let mut offset = size_of::<u32>();

            for url in &sources {
                let len = url.len();

                std::ptr::copy_nonoverlapping(
                    (len as u32).to_le_bytes().as_ptr(),
                    result_ptr.add(offset),
                    size_of::<u32>(),
                );
                offset += size_of::<u32>();

                std::ptr::copy_nonoverlapping(
                    url.as_ptr(),
                    result_ptr.add(offset),
                    len as u32 as usize,
                );
                offset += len;
            }

            debug_assert_eq!(len_required, offset);

            *result_len = len_required as u32;
            REQUIRE_SOURCES
        }
        Err(ImportError::RequireDependencies { dependencies }) => {
            let len_required = dependencies.iter().fold(0, |acc, dep| {
                acc + dep.source.len() + dep.target.len() + size_of::<u32>() * 2
            }) + size_of::<u32>();

            assert!(u32::try_from(len_required).is_ok());

            if *result_len < len_required as u32 {
                *result_len = len_required as u32;
                return BUFFER_IS_TOO_SMALL;
            }

            std::ptr::copy_nonoverlapping(
                (dependencies.len() as u32).to_le_bytes().as_ptr(),
                result_ptr,
                size_of::<u32>(),
            );

            let mut offset = size_of::<u32>();

            for dep in &dependencies {
                for s in [&dep.source, &dep.target] {
                    let len = s.len();

                    std::ptr::copy_nonoverlapping(
                        (len as u32).to_le_bytes().as_ptr(),
                        result_ptr.add(offset),
                        size_of::<u32>(),
                    );
                    offset += size_of::<u32>();

                    std::ptr::copy_nonoverlapping(
                        s.as_ptr(),
                        result_ptr.add(offset),
                        len as u32 as usize,
                    );
                    offset += len;
                }
            }

            debug_assert_eq!(len_required, offset);

            *result_len = len_required as u32;
            REQUIRE_DEPENDENCIES
        }
        Err(ImportError::Other { reason }) => {
            if *result_len < reason.len() as u32 {
                *result_len = reason.len() as u32;
                return BUFFER_IS_TOO_SMALL;
            }

            let error_buf = std::slice::from_raw_parts_mut(result_ptr, reason.len());
            error_buf.copy_from_slice(reason.as_bytes());
            *result_len = reason.len() as u32;
            OTHER_ERROR
        }
    }
}

pub const MAX_EXTENSION_LEN: usize = 16;
pub const MAX_EXTENSION_COUNT: usize = 16;
pub const MAX_FFI_NAME_LEN: usize = 64;
pub const MAX_FORMATS_COUNT: usize = 32;

#[repr(C)]
pub struct ImporterFFI {
    pub importer: *const ImporterOpaque,
    pub import: ImporterImportFn,
    pub name: [u8; MAX_FFI_NAME_LEN],
    pub formats: [[u8; MAX_FFI_NAME_LEN]; MAX_FORMATS_COUNT],
    pub target: [u8; MAX_FFI_NAME_LEN],
    pub extensions: [[u8; MAX_EXTENSION_LEN]; MAX_EXTENSION_COUNT],
}

/// Exporting non thread-safe importers breaks the contract of the FFI.
/// The potential unsoundness is covered by `load_dylib_importers` unsafety.
/// There is no way to guarantee that dynamic library will uphold the contract,
/// making `load_dylib_importers` inevitably unsound.
unsafe impl Send for ImporterFFI {}
unsafe impl Sync for ImporterFFI {}

impl ImporterFFI {
    pub fn new<'a, I>(importer: &'static I) -> Self
    where
        I: Importer,
    {
        let name = importer.name();
        let formats = importer.formats();
        let target = importer.target();
        let extensions = importer.extensions();

        let importer = importer as *const I as *const ImporterOpaque;

        assert!(
            name.len() <= MAX_FFI_NAME_LEN,
            "Importer name should fit into {} bytes",
            MAX_FFI_NAME_LEN
        );
        assert!(
            formats.len() <= MAX_FORMATS_COUNT,
            "Importer should support no more than {} formats",
            MAX_FORMATS_COUNT
        );
        assert!(
            formats.iter().all(|f| f.len() <= MAX_FFI_NAME_LEN),
            "Importer formats should fit into {} bytes",
            MAX_FFI_NAME_LEN
        );
        assert!(
            target.len() <= MAX_FFI_NAME_LEN,
            "Importer target should fit into {} bytes",
            MAX_FFI_NAME_LEN
        );
        assert!(
            extensions.len() < MAX_EXTENSION_COUNT,
            "Importer should support no more than {} extensions",
            MAX_EXTENSION_COUNT,
        );
        assert!(
            extensions.iter().all(|e| e.len() < MAX_EXTENSION_LEN),
            "Importer extensions should fit into {} bytes",
            MAX_EXTENSION_LEN,
        );

        assert!(!name.is_empty(), "Importer name should not be empty");
        assert!(!formats.is_empty(), "Importer formats should not be empty");
        assert!(!target.is_empty(), "Importer target should not be empty");

        assert!(
            !name.contains('\0'),
            "Importer name should not contain '\\0' byte"
        );
        assert!(
            formats.iter().all(|f| !f.contains('\0')),
            "Importer formats should not contain '\\0' byte"
        );
        assert!(
            !target.contains('\0'),
            "Importer target should not contain '\\0' byte"
        );
        assert!(
            extensions.iter().all(|e| !e.contains('\0')),
            "Importer extensions should not contain '\\0' byte"
        );

        let mut name_buf = [0; MAX_FFI_NAME_LEN];
        name_buf[..name.len()].copy_from_slice(name.as_bytes());

        let mut formats_buf = [[0; MAX_FFI_NAME_LEN]; MAX_FORMATS_COUNT];
        for (i, &format) in formats.iter().enumerate() {
            formats_buf[i][..format.len()].copy_from_slice(format.as_bytes());
        }

        let mut target_buf = [0; MAX_FFI_NAME_LEN];
        target_buf[..target.len()].copy_from_slice(target.as_bytes());

        let mut extensions_buf = [[0; MAX_EXTENSION_LEN]; MAX_EXTENSION_COUNT];

        for (i, &extension) in extensions.iter().enumerate() {
            extensions_buf[i][..extension.len()].copy_from_slice(extension.as_bytes());
        }

        ImporterFFI {
            importer,
            import: importer_import_ffi::<I>,
            name: name_buf,
            formats: formats_buf,
            target: target_buf,
            extensions: extensions_buf,
        }
    }
}