use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    time::SystemTime,
};

use argosy_id::AssetId;
use futures::future::BoxFuture;

use super::{AssetData, Source};

pub struct FileSource {
    root: PathBuf,
}

impl Source for FileSource {
    type Error = std::io::Error;

    fn find<'a>(&'a self, _path: &'a str, _asset: &'a str) -> BoxFuture<'a, Option<AssetId>> {
        // Somewhat counter-intuitively, FileSource does not support path-based asset lookup.
        Box::pin(async move { None })
    }

    fn load<'a>(&'a self, id: AssetId) -> BoxFuture<'a, std::io::Result<Option<AssetData>>> {
        let path = self.root.join(id.to_string());

        Box::pin(async move {
            let mut file = match File::open(&path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            };
            let modified = file.metadata().and_then(|m| m.modified()).ok();
            let version = modified.map_or(0, |m| {
                m.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs()
            });

            let len = file.seek(SeekFrom::End(0))?;

            let Ok(len) = usize::try_from(len) else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Asset is too large",
                ));
            };

            file.rewind()?;

            let mut data = Vec::with_capacity(len);
            file.read_to_end(&mut data)?;

            Ok(Some(AssetData {
                bytes: data.into_boxed_slice(),
                version: version,
            }))
        })
    }

    fn update<'a>(
        &'a self,
        id: AssetId,
        version: u64,
    ) -> BoxFuture<'a, Result<Option<AssetData>, Self::Error>> {
        let path = self.root.join(id.to_string());

        Box::pin(async move {
            let mut file = match File::open(&path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            };
            let modified = file.metadata().and_then(|m| m.modified()).ok();
            let new_version = modified.map_or(0, |m| {
                m.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs()
            });

            if new_version <= version {
                return Ok(None);
            }

            let len = file.seek(SeekFrom::End(0))?;

            let Ok(len) = usize::try_from(len) else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Asset is too large",
                ));
            };

            file.rewind()?;

            let mut data = Vec::with_capacity(len);
            file.read_to_end(&mut data)?;

            Ok(Some(AssetData {
                bytes: data.into_boxed_slice(),
                version: new_version,
            }))
        })
    }
}
