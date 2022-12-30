use std::path::PathBuf;

pub trait Sources {
    /// Get data from specified source.
    fn get(&mut self, source: &str) -> Result<Option<PathBuf>, String>;

    fn get_or_append(
        &mut self,
        source: &str,
        missing: &mut Vec<String>,
    ) -> Result<Option<PathBuf>, String> {
        match self.get(source) {
            Err(err) => Err(err),
            Ok(Some(path)) => Ok(Some(path)),
            Ok(None) => {
                missing.push(source.to_owned());
                Ok(None)
            }
        }
    }
}
