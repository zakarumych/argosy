use std::path::Path;

use crate::{Dependencies, Dependency, Sources};

/// Error of `Importer::import` method.
pub enum ImportError {
    /// Importer requires data.
    Requires {
        /// Required sources to build this asset.
        sources: Vec<String>,

        /// Assets this asset depends on.
        dependencies: Vec<Dependency>,
    },

    /// Importer failed to import the asset.
    Other {
        /// Failure reason.
        reason: String,
    },
}

/// Trait for an importer.
pub trait Importer: Send + Sync {
    /// Returns name of the importer
    fn name(&self) -> &str;

    /// Returns source formats importer works with.
    fn formats(&self) -> &[&str];

    /// Returns list of extensions for source formats.
    fn extensions(&self) -> &[&str];

    /// Returns target format importer produces.
    fn target(&self) -> &str;

    /// Reads data from `source` path and writes result at `output` path.
    /// Implementation may request additional sources and dependencies.
    /// If some are missing it **should** return `Err(ImportError::Requires { .. })`
    /// with as much information as possible.
    fn import(
        &self,
        source: &Path,
        output: &Path,
        sources: &mut impl Sources,
        dependencies: &mut impl Dependencies,
    ) -> Result<(), ImportError>;
}
