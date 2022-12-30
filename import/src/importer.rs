use std::path::Path;

use crate::{Dependencies, Dependency, Sources};

/// Result of `Importer::import` method.
pub enum ImportError {
    /// Importer requires data from other sources.
    RequireSources {
        /// URLs relative to source path.
        sources: Vec<String>,
    },

    /// Importer requires following dependencies.
    RequireDependencies { dependencies: Vec<Dependency> },

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

    /// Returns source format importer works with.
    fn formats(&self) -> &[&str];

    /// Returns list of extensions for source formats.
    fn extensions(&self) -> &[&str];

    /// Returns target format importer produces.
    fn target(&self) -> &str;

    /// Reads data from `source` path and writes result at `output` path.
    fn import(
        &self,
        source: &Path,
        output: &Path,
        sources: &mut dyn Sources,
        dependencies: &mut dyn Dependencies,
    ) -> Result<(), ImportError>;
}
