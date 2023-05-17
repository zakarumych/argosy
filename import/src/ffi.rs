use std::{marker::PhantomData, mem::size_of, path::PathBuf};

#[cfg(any(unix, target_os = "wasi"))]
use std::ffi::{OsStr, OsString};

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};

#[cfg(target_os = "wasi")]
use std::os::wasi::ffi::{OsStrExt, OsStringExt};

#[cfg(windows)]
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
};

use argosy_id::AssetId;

use crate::{
    dependencies::Dependencies,
    importer::{ImportError, Importer},
    sources::Sources,
};

const PATH_BUF_LEN_START: usize = 1024;
pub const ANY_BUF_LEN_LIMIT: usize = 65536;

pub const REQUIRES: i32 = 1;
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

unsafe extern "C" fn dependencies_get_ffi<D: Dependencies>(
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

    let d = &mut *(dependencies as *mut D);

    match d.get(source, target) {
        None => return NOT_FOUND,
        Some(id) => {
            std::ptr::write(id_ptr, id.value().get());
            return SUCCESS;
        }
    }
}

pub struct DependenciesFFI<'a> {
    pub opaque: *mut DependenciesOpaque,
    pub get: DependenciesGetFn,
    marker: PhantomData<&'a ()>,
}

impl<'a> DependenciesFFI<'a> {
    pub fn new<D: Dependencies>(dependencies: &'a mut D) -> Self {
        DependenciesFFI {
            opaque: (dependencies as *mut D) as *mut DependenciesOpaque,
            get: dependencies_get_ffi::<D>,
            marker: PhantomData,
        }
    }
}

impl Dependencies for DependenciesFFI<'_> {
    fn get(&mut self, source: &str, target: &str) -> Option<AssetId> {
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
                None => panic!("Null AssetId returned from `Dependencies::get`"),
                Some(id) => Some(id),
            },
            NOT_FOUND => None,
            NOT_UTF8 => panic!("Source is not UTF8 while stored in `str`"),
            _ => panic!("Unexpected return code from `Sources::get` FFI: {}", result),
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

unsafe extern "C" fn sources_get_ffi<'a, S: Sources>(
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

    let f = &mut *(sources as *mut S);

    match f.get(source) {
        None => return NOT_FOUND,
        Some(path) => {
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

pub struct SourcesFFI<'a> {
    pub opaque: *mut SourcesOpaque,
    pub get: SourcesGetFn,
    marker: PhantomData<&'a ()>,
}

impl<'a> SourcesFFI<'a> {
    pub fn new<S: Sources>(sources: &'a mut S) -> Self {
        SourcesFFI {
            opaque: sources as *const S as _,
            get: sources_get_ffi::<S>,
            marker: PhantomData,
        }
    }
}

impl Sources for SourcesFFI<'_> {
    fn get(&mut self, source: &str) -> Option<PathBuf> {
        let mut path_buf = vec![0; PATH_BUF_LEN_START];
        let mut path_len = PATH_BUF_LEN_START as u32;
        let mut result = BUFFER_IS_TOO_SMALL;

        while result == BUFFER_IS_TOO_SMALL {
            if path_len > ANY_BUF_LEN_LIMIT as u32 {
                panic!(
                    "Source path does not fit into limit '{}', '{}' required",
                    ANY_BUF_LEN_LIMIT, path_len
                );
            }

            path_buf.resize(path_len as usize, 0);

            result = unsafe {
                (self.get)(
                    self.opaque,
                    source.as_ptr(),
                    source.len() as u32,
                    path_buf.as_mut_ptr(),
                    &mut path_len,
                )
            };
        }

        path_buf.truncate(path_len as usize);

        match result {
            SUCCESS => {
                #[cfg(any(unix, target_os = "wasi"))]
                let path = OsString::from_vec(path_buf).into();

                #[cfg(windows)]
                let path = OsString::from_wide(&path_buf).into();

                Some(path)
            }
            NOT_FOUND => None,
            NOT_UTF8 => panic!("Source is not UTF8 while stored in `str`"),
            _ => panic!("Unexpected return code from `Sources::get` FFI: {}", result),
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

unsafe extern "C" fn importer_import_ffi<I: Importer>(
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
) -> i32 {
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
        marker: PhantomData,
    };

    let mut dependencies = DependenciesFFI {
        opaque: dependencies,
        get: dependencies_get,
        marker: PhantomData,
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
        Err(ImportError::Requires {
            sources,
            dependencies,
        }) => {
            let len_required = sources
                .iter()
                .map(|s| s.len() + size_of::<u32>())
                .chain(
                    dependencies
                        .iter()
                        .map(|d| d.source.len() + d.target.len() + size_of::<[u32; 2]>()),
                )
                .sum::<usize>()
                + size_of::<[u32; 2]>();

            assert!(u32::try_from(len_required).is_ok());

            if *result_len < len_required as u32 {
                *result_len = len_required as u32;
                return BUFFER_IS_TOO_SMALL;
            }

            let result = std::slice::from_raw_parts_mut(result_ptr, len_required);
            let mut offset = 0;

            write_u32(result, &mut offset, source.len() as u32);
            for source in sources {
                write_slice(result, &mut offset, source.as_bytes());
            }

            write_u32(result, &mut offset, dependencies.len() as u32);
            for dependency in dependencies {
                write_slice(result, &mut offset, dependency.source.as_bytes());
                write_slice(result, &mut offset, dependency.target.as_bytes());
            }

            *result_len = len_required as u32;
            REQUIRES
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

fn write_u32(buffer: &mut [u8], offset: &mut usize, value: u32) {
    buffer[*offset..][..4].copy_from_slice(&value.to_le_bytes());
    *offset += 4;
}

fn write_slice(buffer: &mut [u8], offset: &mut usize, value: &[u8]) {
    write_u32(buffer, offset, value.len() as u32);
    buffer[*offset..][..4].copy_from_slice(value);
    *offset += value.len();
}
