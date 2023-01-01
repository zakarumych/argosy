use argosy_id::AssetId;

#[derive(Debug)]
pub struct Dependency {
    pub source: String,
    pub target: String,
}

pub trait Dependencies {
    /// Returns dependency id.
    fn get(&mut self, source: &str, target: &str) -> Result<Option<AssetId>, String>;

    fn get_or_append(
        &mut self,
        source: &str,
        target: &str,
        missing: &mut Vec<Dependency>,
    ) -> Result<Option<AssetId>, String> {
        match self.get(source, target) {
            Err(err) => Err(err),
            Ok(Some(id)) => Ok(Some(id)),
            Ok(None) => {
                missing.push(Dependency {
                    source: source.to_owned(),
                    target: target.to_owned(),
                });
                Ok(None)
            }
        }
    }
}
