use argosy_id::AssetId;

/// Single dependency for a asset.
#[derive(Debug)]
pub struct Dependency {
    /// Source path.
    pub source: String,

    /// Target format.
    pub target: String,
}

/// Provides access to asset dependencies.
/// Converts source and target to asset id.
pub trait Dependencies {
    /// Returns dependency id.
    /// If dependency is not available, returns `None`.
    fn get(&mut self, source: &str, target: &str) -> Option<AssetId>;

    /// Returns dependency id.
    /// If dependency is not available,
    /// append it to the missing list and returns `None`.
    fn get_or_append(
        &mut self,
        source: &str,
        target: &str,
        missing: &mut Vec<Dependency>,
    ) -> Option<AssetId> {
        match self.get(source, target) {
            None => {
                missing.push(Dependency {
                    source: source.to_owned(),
                    target: target.to_owned(),
                });
                None
            }
            Some(id) => Some(id),
        }
    }
}

impl<D: ?Sized> Dependencies for &mut D
where
    D: Dependencies,
{
    fn get(&mut self, source: &str, target: &str) -> Option<AssetId> {
        (*self).get(source, target)
    }
}
