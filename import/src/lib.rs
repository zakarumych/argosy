//! Contains everything that is required to create argosy importers library.
//!
//!
//! # Usage
//!
//! ```
//! struct FooImporter;
//!
//! impl argosy_import::Importer for FooImporter {
//!     fn name(&self) -> &str {
//!         "Foo importer"
//!     }
//!
//!     fn formats(&self) -> &[&str] {
//!         &["foo"]
//!     }
//!
//!     fn target(&self) -> &str {
//!         "foo"
//!     }
//!
//!     fn extensions(&self) -> &[&str] {
//!         &["json"]
//!     }
//!
//!     fn import(
//!         &self,
//!         source: &std::path::Path,
//!         output: &std::path::Path,
//!         _sources: &mut dyn argosy_import::Sources,
//!         _dependencies: &mut dyn argosy_import::Dependencies,
//!     ) -> Result<(), argosy_import::ImportError> {
//!         match std::fs::copy(source, output) {
//!           Ok(_) => Ok(()),
//!           Err(err) => Err(argosy_import::ImportError::Other { reason: "SOMETHING WENT WRONG".to_owned() }),
//!         }
//!     }
//! }
//!
//!
//! // Define all required exports.
//! argosy_import::make_argosy_importers_library! {
//!     // Each <expr;> must have type &'static I where I: Importer
//!     &FooImporter;
//! }
//! ```

mod dependencies;
mod ffi;
mod importer;
mod sources;

#[cfg(feature = "libloading")]
pub mod loading;

pub use ffi::ImporterFFI;

pub use self::{
    dependencies::{Dependencies, Dependency},
    importer::{ImportError, Importer},
    sources::Sources,
};

/// Helper function to emit an error if sources or dependencies are missing.
pub fn ensure(sources: Vec<String>, dependencies: Vec<Dependency>) -> Result<(), ImportError> {
    if sources.is_empty() && dependencies.is_empty() {
        Ok(())
    } else {
        Err(ImportError::Requires {
            sources,
            dependencies,
        })
    }
}

pub fn version() -> u32 {
    let version = env!("CARGO_PKG_VERSION_MINOR");
    let version = version.parse().unwrap();
    assert_ne!(
        version,
        u32::MAX,
        "Minor version hits u32::MAX. Oh no. Upgrade to u64",
    );
    version
}

pub const MAGIC: u32 = u32::from_le_bytes(*b"TRES");

/// Defines exports required for an importers library.
/// Accepts repetition of importer expressions of type [`&'static impl Importer`] delimited by ';'.
///
/// This macro must be used exactly once in a library crate.
/// The library must be compiled as a dynamic library to be loaded by the argosy.
#[macro_export]
macro_rules! make_argosy_importers_library {
    ($($importer:expr);* $(;)?) => {
        #[no_mangle]
        pub static ARGOSY_DYLIB_MAGIC: u32 = $crate::MAGIC;

        #[no_mangle]
        pub unsafe extern "C" fn argosy_importer_ffi_version_minor() -> u32 {
            $crate::version()
        }

        #[no_mangle]
        pub unsafe extern "C" fn argosy_export_importers(buffer: *mut $crate::ImporterFFI, mut cap: u32) -> u32 {
            let mut len = 0;
            $(
                if cap > 0 {
                    core::ptr::write(buffer.add(len as usize), $crate::ImporterFFI::new($importer));
                    cap -= 1;
                }
                len += 1;
            )*
            len
        }
    };
}
