use std::{
    fs::File,
    io::Write,
    mem::size_of_val,
    path::{Path, PathBuf},
    time::SystemTime,
};

use base64::{
    alphabet::URL_SAFE,
    engine::fast_portable::{FastPortable, NO_PAD},
};
use eyre::WrapErr;
use hashbrown::{hash_map::RawEntryMut, HashMap};
use url::Url;

use crate::{scheme::Scheme, temp::Temporaries};

/// Fetches and caches sources.
/// Saves remote sources to temporaries.
pub struct Sources {
    feched: HashMap<Url, (PathBuf, bool)>,
}

impl Sources {
    pub fn new() -> Self {
        Sources {
            feched: HashMap::new(),
        }
    }

    pub fn get(&self, source: &Url) -> Option<(&Path, Option<SystemTime>)> {
        let (path, local) = self.feched.get(source)?;
        if *local {
            let modified = path.metadata().ok()?.modified().ok()?;
            Some((path, Some(modified)))
        } else {
            Some((path, None))
        }
    }

    pub async fn fetch(
        &mut self,
        temporaries: &mut Temporaries<'_>,
        source: &Url,
    ) -> eyre::Result<(&Path, Option<SystemTime>)> {
        match self.feched.raw_entry_mut().from_key(source) {
            RawEntryMut::Occupied(entry) => {
                let (path, local) = entry.into_mut();
                if *local {
                    let modified = path.metadata()?.modified()?;
                    Ok((path, Some(modified)))
                } else {
                    Ok((path, None))
                }
            }
            RawEntryMut::Vacant(entry) => match source.scheme().parse() {
                Ok(Scheme::File) => {
                    let path = source
                        .to_file_path()
                        .map_err(|()| eyre::eyre!("Invalid file: URL"))?;

                    let modified = path.metadata()?.modified()?;

                    tracing::debug!("Fetching file '{}' ('{}')", source, path.display());
                    let (_, (path, _)) = entry.insert(source.clone(), (path, true));

                    Ok((path, Some(modified)))
                }
                Ok(Scheme::Data) => {
                    let data_start = source.as_str()[size_of_val("data:")..]
                        .find(',')
                        .ok_or_else(|| eyre::eyre!("Invalid data URL"))?
                        + 1
                        + size_of_val("data:");
                    let data = &source.as_str()[data_start..];

                    let temp = temporaries.make_temporary();
                    let mut file = File::create(&temp)
                        .wrap_err("Failed to create temporary file to store data URL content")?;

                    if source.as_str()[..data_start].ends_with(";base64,") {
                        let decoded =
                            base64::decode_engine(data, &FastPortable::from(&URL_SAFE, NO_PAD))
                                .wrap_err("Failed to decode base64 data url")?;

                        file.write_all(&decoded).wrap_err_with(|| {
                            format!(
                                "Failed to write data URL content to temporary file '{}'",
                                temp.display(),
                            )
                        })?;
                    } else {
                        file.write_all(data.as_bytes()).wrap_err_with(|| {
                            format!(
                                "Failed to write data URL content to temporary file '{}'",
                                temp.display(),
                            )
                        })?;
                    }

                    let (_, (path, _)) = entry.insert(source.clone(), (temp, false));
                    Ok((path, None))
                }
                Err(_) => Err(eyre::eyre!("Unsupported scheme '{}'", source.scheme())),
            },
        }
    }
}
