use std::{
    error::Error,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};

use argosy_id::AssetId;
use eyre::WrapErr;
use hashbrown::HashMap;
use url::Url;

use crate::{scheme::Scheme, sha256::Sha256Hash};

const PREFIX_STARTING_LEN: usize = 8;
const EXTENSION: &'static str = "treasure";
const DOT_EXTENSION: &'static str = ".treasure";

#[derive(serde::Serialize, serde::Deserialize)]
pub struct AssetMeta {
    id: AssetId,

    /// Imported asset file hash.
    sha256: Sha256Hash,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    format: Option<String>,

    #[serde(skip_serializing_if = "prefix_is_default", default = "default_prefix")]
    prefix: usize,

    #[serde(skip_serializing_if = "suffix_is_zero", default)]
    suffix: u64,

    // Array of dependencies of this asset.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    dependencies: Vec<AssetId>,

    // Key is URL, value is last modified time.
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    sources: HashMap<String, SystemTime>,
}

fn prefix_is_default(prefix: &usize) -> bool {
    *prefix == PREFIX_STARTING_LEN
}

fn default_prefix() -> usize {
    PREFIX_STARTING_LEN
}

fn suffix_is_zero(suffix: &u64) -> bool {
    *suffix == 0
}

impl AssetMeta {
    /// Creates new asset metadata.
    /// Puts ouput to the artifacs directory.
    ///
    /// This function is when new asset is imported.
    ///
    /// `output` contain temporary path to imported asset artifact.
    /// `artifacts` is path to artifact directory.
    ///
    /// Filename of the output gets chosen using first N characters of the sha512 hash.
    /// Where N is the minimal length required to avoid collisions between files with same hash prefixes.
    /// It can also get a suffix if there is a complete hash collision.
    ///
    /// If artifact with the same hash already exists in the `artifacts` directory,
    /// it will be shared between assets.
    pub fn new(
        id: AssetId,
        format: Option<String>,
        sources: Vec<(String, SystemTime)>,
        dependencies: Vec<AssetId>,
        output: &Path,
        artifacts: &Path,
    ) -> eyre::Result<Self> {
        let sha256 = Sha256Hash::file_hash(output).wrap_err_with(|| {
            format!(
                "Failed to calculate hash of the file '{}'",
                output.display()
            )
        })?;

        let hex = format!("{:x}", sha256);

        let (prefix, suffix) = with_path_candidates(
            &hex,
            artifacts,
            move |prefix, suffix, path| -> eyre::Result<_> {
                match path.metadata() {
                    Err(_) => {
                        // Artifact file does not exists.
                        // This is the most common case.
                        std::fs::rename(output, &path).wrap_err_with(|| {
                            format!(
                                "Failed to rename output file '{}' to artifact file '{}'",
                                output.display(),
                                path.display()
                            )
                        })?;

                        Ok(Some((prefix, suffix)))
                    }
                    Ok(meta) if meta.is_file() => {
                        // Artifacto file already exists.
                        // Check if it is the same file or just a prefix collision.
                        let eq = files_eq(output, &path).wrap_err_with(|| {
                            format!(
                                "Failed to compare artifact file '{}' and new asset output '{}'",
                                path.display(),
                                output.display(),
                            )
                        })?;

                        if eq {
                            tracing::warn!("Artifact for asset '{}' is already in storage", id);

                            if let Err(err) = std::fs::remove_file(output) {
                                tracing::error!(
                                    "Failed to remove duplicate artifact file '{}'. {:#}",
                                    err,
                                    output.display()
                                );
                            }

                            Ok(Some((prefix, suffix)))
                        } else {
                            // Prefixes are the same.
                            // Try longer prefix.
                            tracing::debug!("Artifact path collision");
                            Ok(None)
                        }
                    }
                    Ok(_) => {
                        // Path is occupied by directory.
                        // This should never be caused by the store itself.
                        // But it can be caused by user and is not treated as an error.
                        tracing::warn!(
                            "Artifacts storage occupied by non-file entity '{}'",
                            path.display()
                        );
                        Ok(None)
                    }
                }
            },
        )?;

        Ok(AssetMeta {
            id,
            format,
            sha256,
            prefix,
            suffix,
            sources: sources.into_iter().collect(),
            dependencies,
        })
    }

    pub fn id(&self) -> AssetId {
        self.id
    }

    pub fn format(&self) -> Option<&str> {
        self.format.as_deref()
    }

    pub fn needs_reimport(&self, base: &Url) -> bool {
        for (url, last_modified) in &self.sources {
            let url = match base.join(url) {
                Err(err) => {
                    tracing::error!(
                        "Failed to figure out source URL from base: {} and source: {}. {:#}. Asset can be outdated",
                        base,
                        url,
                        err,
                    );
                    continue;
                }
                Ok(url) => url,
            };

            match url.scheme().parse() {
                Ok(Scheme::File) => {
                    let path = match url.to_file_path() {
                        Err(()) => {
                            tracing::error!("Invalid file URL");
                            continue;
                        }
                        Ok(path) => path,
                    };

                    let modified = match path.metadata().and_then(|meta| meta.modified()) {
                        Err(err) => {
                            tracing::error!(
                                "Failed to check how new the source file is. {:#}",
                                err
                            );
                            continue;
                        }
                        Ok(modified) => modified,
                    };

                    if modified < *last_modified {
                        tracing::warn!("Source file is older than when asset was imported. Could be clock change. Reimort just in case");
                        return true;
                    }

                    if modified > *last_modified {
                        tracing::debug!("Source file was updated");
                        return true;
                    }
                }
                Ok(Scheme::Data) => continue,
                Err(_) => tracing::error!("Unsupported scheme: '{}'", url.scheme()),
            }
        }

        false
    }

    /// Returns path to the artifact.
    pub fn artifact_path(&self, artifacts: &Path) -> PathBuf {
        let hex = format!("{:x}", self.sha256);
        let prefix = &hex[..self.prefix];

        match self.suffix {
            0 => artifacts.join(prefix),
            suffix => artifacts.join(format!("{}:{}", prefix, suffix)),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Error: '{}' while trying to canonicalize path '{}'", error, path.display())]
struct CanonError {
    #[source]
    error: std::io::Error,
    path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to convert path '{}' to URL", path.display())]
struct UrlFromPathError {
    path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
#[error("Error: '{}' with file: '{}'", error, path.display())]
struct FileError<E: Error> {
    #[source]
    error: E,
    path: PathBuf,
}

/// Data attached to single asset source.
/// It may include several assets.
/// If attached to external source outside store directory
/// then it is stored together with artifacts by URL hash.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SourceMeta {
    url: Url,
    assets: HashMap<String, AssetMeta>,
}

impl SourceMeta {
    /// Finds and returns meta for the source URL.
    /// Creates new file if needed.
    pub fn new(source: &Url, base: &Path, external: &Path) -> eyre::Result<SourceMeta> {
        let (meta_path, is_external) = get_meta_path(source, base, external)?;

        if is_external {
            SourceMeta::new_external(&meta_path, source)
        } else {
            SourceMeta::new_local(&meta_path)
        }
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn is_local_meta_path(meta_path: &Path) -> bool {
        meta_path.extension().map_or(false, |e| e == EXTENSION)
    }

    pub fn new_local(meta_path: &Path) -> eyre::Result<SourceMeta> {
        SourceMeta::read_local(meta_path, true)
    }

    pub fn open_local(meta_path: &Path) -> eyre::Result<SourceMeta> {
        SourceMeta::read_local(meta_path, false)
    }

    fn read_local(meta_path: &Path, allow_missing: bool) -> eyre::Result<Self> {
        let source_path = meta_path.with_extension("");
        let url = Url::from_file_path(&source_path)
            .map_err(|()| UrlFromPathError { path: source_path })?;

        match std::fs::read(meta_path) {
            Err(err) if allow_missing && err.kind() == std::io::ErrorKind::NotFound => {
                Ok(SourceMeta {
                    url,
                    assets: HashMap::new(),
                })
            }
            Err(err) => Err(FileError {
                error: err,
                path: meta_path.to_owned(),
            })
            .wrap_err("Meta read failed"),
            Ok(data) => {
                let assets = toml::from_slice(&data)
                    .map_err(|err| FileError {
                        error: err,
                        path: meta_path.to_owned(),
                    })
                    .wrap_err("Meta read failed")?;
                Ok(SourceMeta { url, assets })
            }
        }
    }

    pub fn new_external(meta_path: &Path, source: &Url) -> eyre::Result<SourceMeta> {
        match std::fs::read(meta_path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(SourceMeta {
                url: source.clone(),
                assets: HashMap::new(),
            }),
            Err(err) => Err(FileError {
                error: err,
                path: meta_path.to_owned(),
            })
            .wrap_err("Meta read failed"),
            Ok(data) => {
                let assets = toml::from_slice(&data)
                    .map_err(|err| FileError {
                        error: err,
                        path: meta_path.to_owned(),
                    })
                    .wrap_err("Meta read failed")?;
                Ok(SourceMeta {
                    url: source.clone(),
                    assets,
                })
            }
        }
    }

    pub fn open_external(meta_path: &Path) -> eyre::Result<SourceMeta> {
        match std::fs::read(meta_path) {
            Err(err) => Err(FileError {
                error: err,
                path: meta_path.to_owned(),
            })
            .wrap_err("Meta read failed"),
            Ok(data) => {
                let meta = toml::from_slice(&data)
                    .map_err(|err| FileError {
                        error: err,
                        path: meta_path.to_owned(),
                    })
                    .wrap_err("Meta read failed")?;
                Ok(meta)
            }
        }
    }

    pub fn get_asset(&self, target: &str) -> Option<&AssetMeta> {
        self.assets.get(target)
    }

    pub fn assets(&self) -> impl Iterator<Item = (&str, &AssetMeta)> + '_ {
        self.assets.iter().map(|(target, meta)| (&**target, meta))
    }

    pub fn add_asset(
        &mut self,
        target: String,
        asset: AssetMeta,
        base: &Path,
        external: &Path,
    ) -> eyre::Result<()> {
        self.assets.insert(target, asset);

        let (meta_path, is_external) = get_meta_path(&self.url, base, external)?;
        if is_external {
            self.write_with_url_to(&meta_path)?;
        } else {
            self.write_to(&meta_path)?;
        }
        Ok(())
    }

    fn write_to(&self, path: &Path) -> eyre::Result<()> {
        let data = toml::to_string_pretty(&self.assets)
            .map_err(|err| FileError {
                error: err,
                path: path.to_owned(),
            })
            .wrap_err("Meta write failed")?;
        std::fs::write(path, data.as_bytes())
            .map_err(|err| FileError {
                error: err,
                path: path.to_owned(),
            })
            .wrap_err("Meta write failed")?;
        Ok(())
    }

    fn write_with_url_to(&self, path: &Path) -> eyre::Result<()> {
        let data = toml::to_string_pretty(self)
            .map_err(|err| FileError {
                error: err,
                path: path.to_owned(),
            })
            .wrap_err("Meta write failed")?;
        std::fs::write(path, data.as_bytes())
            .map_err(|err| FileError {
                error: err,
                path: path.to_owned(),
            })
            .wrap_err("Meta write failed")?;
        Ok(())
    }
}

fn files_eq(lhs: &Path, rhs: &Path) -> std::io::Result<bool> {
    let mut lhs = File::open(lhs)?;
    let mut rhs = File::open(rhs)?;

    let lhs_size = lhs.seek(SeekFrom::End(0))?;
    let rhs_size = rhs.seek(SeekFrom::End(0))?;

    if lhs_size != rhs_size {
        return Ok(false);
    }

    lhs.seek(SeekFrom::Start(0))?;
    rhs.seek(SeekFrom::Start(0))?;

    let mut buffer_lhs = [0; 16536];
    let mut buffer_rhs = [0; 16536];

    loop {
        let read = lhs.read(&mut buffer_lhs)?;
        if read == 0 {
            return Ok(true);
        }
        rhs.read_exact(&mut buffer_rhs[..read])?;

        if buffer_lhs[..read] != buffer_rhs[..read] {
            return Ok(false);
        }
    }
}

/// Finds and returns meta for the source URL.
/// Creates new file if needed.
fn get_meta_path(source: &Url, base: &Path, external: &Path) -> eyre::Result<(PathBuf, bool)> {
    if source.scheme() == "file" {
        match source.to_file_path() {
            Ok(path) => {
                let path =
                    dunce::canonicalize(&path).map_err(|err| CanonError { error: err, path })?;

                if path.starts_with(base) {
                    // Files inside `base` directory has meta attached to them as sibling file with `.treasure` extension added.

                    let mut filename = path.file_name().unwrap_or("".as_ref()).to_owned();
                    filename.push(DOT_EXTENSION);

                    let path = path.with_file_name(filename);
                    return Ok((path, false));
                }
            }
            Err(()) => {}
        }
    }

    std::fs::create_dir_all(external).wrap_err_with(|| {
        format!(
            "Failed to create external directory '{}'",
            external.display()
        )
    })?;

    let hash = Sha256Hash::new(source.as_str());
    let hex = format!("{:x}", hash);

    with_path_candidates(&hex, external, |_prefix, _suffix, path| {
        match path.metadata() {
            Err(_) => {
                // Not exists. Let's try to occupy.
                Ok(Some((path, true)))
            }
            Ok(md) => {
                if md.is_file() {
                    match SourceMeta::open_external(&path) {
                        Err(_) => {
                            tracing::error!(
                                "Failed to open existing source metadata at '{}'",
                                path.display()
                            );
                        }
                        Ok(meta) => {
                            if meta.url == *source {
                                return Ok(Some((path, true)));
                            }
                        }
                    }
                }
                Ok(None)
            }
        }
    })
}

fn with_path_candidates<T, E>(
    hex: &str,
    base: &Path,
    mut f: impl FnMut(usize, u64, PathBuf) -> Result<Option<T>, E>,
) -> Result<T, E> {
    use std::fmt::Write;

    for len in PREFIX_STARTING_LEN..=hex.len() {
        let path = base.join(&hex[..len]);

        match f(len, 0, path) {
            Ok(None) => {}
            Ok(Some(ok)) => return Ok(ok),
            Err(err) => return Err(err),
        }
    }

    // Rarely needed.
    let mut name = hex.to_owned();

    for suffix in 0u64.. {
        name.truncate(hex.len());
        write!(name, ":{}", suffix).unwrap();

        let path = base.join(&name);

        match f(hex.len(), suffix, path) {
            Ok(None) => {}
            Ok(Some(ok)) => return Ok(ok),
            Err(err) => return Err(err),
        }
    }

    unreachable!()
}
