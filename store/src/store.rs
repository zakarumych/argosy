use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    time::SystemTime,
};

use asset_influx_id::AssetId;
use asset_influx_import::{loading::LoadingError, ImportError, Importer};
use eyre::WrapErr;
use hashbrown::{HashMap, HashSet};
use parking_lot::RwLock;
use url::Url;

use crate::{
    id_gen::IdGen,
    importer::Importers,
    meta::{AssetMeta, SourceMeta},
    sources::Sources,
    temp::Temporaries,
};

pub const ASSET_INFLUX_META_NAME: &'static str = "influx.toml";

const DEFAULT_AUX: &'static str = "influx";
const DEFAULT_ARTIFACTS: &'static str = "artifacts";
const DEFAULT_EXTERNAL: &'static str = "external";
const MAX_ITEM_ATTEMPTS: u32 = 1024;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct StoreInfo {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub artifacts: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub external: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub temp: Option<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub importers: Vec<PathBuf>,
}

impl Default for StoreInfo {
    fn default() -> Self {
        StoreInfo::new(None, None, None, &[])
    }
}

impl StoreInfo {
    pub fn write(&self, path: &Path) -> eyre::Result<()> {
        let meta = toml::to_string_pretty(self).wrap_err("Failed to serialize metadata")?;
        std::fs::write(path, &meta)
            .wrap_err_with(|| format!("Failed to write metadata file '{}'", path.display()))?;
        Ok(())
    }

    pub fn read(path: &Path) -> eyre::Result<Self> {
        let err_ctx = || format!("Failed to read metadata file '{}'", path.display());

        let meta = std::fs::read(path).wrap_err_with(err_ctx)?;
        let meta: StoreInfo = toml::from_slice(&meta).wrap_err_with(err_ctx)?;
        Ok(meta)
    }

    pub fn new(
        artifacts: Option<&Path>,
        external: Option<&Path>,
        temp: Option<&Path>,
        importers: &[&Path],
    ) -> Self {
        let artifacts = artifacts.map(Path::to_owned);
        let external = external.map(Path::to_owned);
        let temp = temp.map(Path::to_owned);
        let importers = importers.iter().copied().map(|p| p.to_owned()).collect();

        StoreInfo {
            artifacts,
            external,
            temp,
            importers,
        }
    }
}

#[derive(Clone)]
struct AssetItem {
    source: Url,
    format: Option<String>,
    target: String,
}

pub struct Store {
    base: PathBuf,
    base_url: Url,
    artifacts_base: PathBuf,
    external: PathBuf,
    temp: PathBuf,
    importers: Importers,

    artifacts: RwLock<HashMap<AssetId, AssetItem>>,
    scanned: RwLock<bool>,
    id_gen: IdGen,
}

impl Store {
    /// Find and open store in ancestors of specified directory.
    #[tracing::instrument]
    pub fn find(path: &Path) -> eyre::Result<Self> {
        let path = dunce::canonicalize(path)?;
        let err = format!(
            "Failed to find `{}` in ancestors of {}",
            ASSET_INFLUX_META_NAME,
            path.display(),
        );
        let meta_path = find_asset_influx_info(path).ok_or_else(|| eyre::eyre!(err))?;

        Store::open(&meta_path)
    }

    /// Find and open store in ancestors of current directory.
    #[tracing::instrument]
    pub fn find_current() -> eyre::Result<Self> {
        let cwd = std::env::current_dir().wrap_err("Failed to get current directory")?;
        Store::find(&cwd)
    }

    /// Open store database at specified path.
    #[tracing::instrument]
    pub fn open(path: &Path) -> eyre::Result<Self> {
        let meta = StoreInfo::read(path)?;
        let base = path.parent().unwrap().to_owned();

        Self::new(&base, meta)
    }

    pub fn new(base: &Path, meta: StoreInfo) -> eyre::Result<Self> {
        let base = dunce::canonicalize(base).wrap_err_with(|| {
            eyre::eyre!("Failed to canonicalize base path '{}'", base.display())
        })?;
        let base_url = Url::from_directory_path(&base)
            .map_err(|()| eyre::eyre!("'{}' is invalid base path", base.display()))?;

        let artifacts = base.join(
            meta.artifacts
                .unwrap_or_else(|| Path::new(DEFAULT_AUX).join(DEFAULT_ARTIFACTS)),
        );

        let external = base.join(
            meta.external
                .unwrap_or_else(|| Path::new(DEFAULT_AUX).join(DEFAULT_EXTERNAL)),
        );

        let temp = meta
            .temp
            .map_or_else(std::env::temp_dir, |path| base.join(path));

        let mut importers = Importers::new();

        for lib_path in &meta.importers {
            let lib_path = base.join(lib_path);

            unsafe {
                // # Safety: Nope.
                // There is no way to make this safe.
                // But it is unlikely to cause problems by accident.
                if let Err(err) = importers.load_dylib_importers(&lib_path) {
                    tracing::error!(
                        "Failed to load importers from '{}'. {:#}",
                        lib_path.display(),
                        err
                    );
                }
            }
        }

        Ok(Store {
            base,
            base_url,
            artifacts_base: artifacts,
            external,
            temp,
            importers,
            artifacts: RwLock::new(HashMap::new()),
            scanned: RwLock::new(false),
            id_gen: IdGen::new(),
        })
    }

    /// Loads importers from dylib.
    /// There is no possible way to guarantee that dylib does not break safety contracts.
    /// Some measures to ensure safety are taken.
    /// Providing dylib from which importers will be successfully loaded and then cause an UB should only be possible on purpose.
    #[tracing::instrument(skip(self))]
    pub unsafe fn register_importers_lib(&mut self, lib_path: &Path) -> Result<(), LoadingError> {
        self.importers.load_dylib_importers(lib_path)
    }

    /// Adds importer to the store.
    #[tracing::instrument(skip_all)]
    pub fn register_importer(&mut self, importer: impl Importer + 'static) {
        self.importers.register_importer(importer)
    }

    /// Import an asset.
    #[tracing::instrument(skip(self))]
    pub async fn store(
        &self,
        source: &str,
        format: Option<&str>,
        target: &str,
    ) -> eyre::Result<(AssetId, PathBuf)> {
        let source = self.base_url.join(source).wrap_err_with(|| {
            format!(
                "Failed to construct URL from base '{}' and source '{}'",
                self.base_url, source
            )
        })?;

        self.store_url(source, format, target).await
    }

    /// Import an asset.
    #[tracing::instrument(skip(self))]
    pub async fn store_url(
        &self,
        source: Url,
        format: Option<&str>,
        target: &str,
    ) -> eyre::Result<(AssetId, PathBuf)> {
        let mut temporaries = Temporaries::new(&self.temp);
        let mut sources = Sources::new();

        let base = &self.base;
        let artifacts = &self.artifacts_base;
        let external = &self.external;
        let importers = &self.importers;

        struct StackItem {
            /// Source URL.
            source: Url,

            /// Source format name.
            format: Option<String>,

            /// Target format name.
            target: String,

            /// Attempt counter to break infinite loops.
            attempt: u32,

            /// Sources requested by importer.
            /// Relative to `source`.
            sources: HashMap<Url, SystemTime>,

            /// Dependencies requested by importer.
            dependencies: HashSet<AssetId>,
        }

        let mut stack = Vec::new();
        stack.push(StackItem {
            source,
            format: format.map(str::to_owned),
            target: target.to_owned(),
            attempt: 0,
            sources: HashMap::new(),
            dependencies: HashSet::new(),
        });

        loop {
            // tokio::time::sleep(Duration::from_secs(1)).await;

            let item = stack.last_mut().unwrap();
            item.attempt += 1;

            let mut meta = SourceMeta::new(&item.source, &self.base, &self.external)
                .wrap_err("Failed to fetch source meta")?;

            if let Some(asset) = meta.get_asset(&item.target) {
                if asset.needs_reimport(&self.base_url) {
                    tracing::debug!(
                        "'{}' '{:?}' '{}' reimporting",
                        item.source,
                        item.format,
                        item.target
                    );
                } else {
                    match &item.format {
                        None => tracing::debug!("{} @ '{}'", item.target, item.source),
                        Some(format) => {
                            tracing::debug!("{} as {} @ '{}'", item.target, format, item.source)
                        }
                    }

                    stack.pop().unwrap();
                    if stack.is_empty() {
                        return Ok((asset.id(), asset.artifact_path(&self.artifacts_base)));
                    }
                    continue;
                }
            }

            let importer =
                importers.guess(item.format.as_deref(), url_ext(&item.source), &item.target)?;

            let importer = importer.ok_or_else(|| {
                eyre::eyre!(
                    "Failed to find importer '{} -> {}' for asset '{}'",
                    item.format.as_deref().unwrap_or("<undefined>"),
                    item.target,
                    item.source,
                )
            })?;

            // Fetch source file.
            let (source_path, modified) = sources.fetch(&mut temporaries, &item.source).await?;
            let source_path = source_path.to_owned();

            let output_path = temporaries.make_temporary();

            struct Fn<F>(F);

            impl<F> asset_influx_import::Sources for Fn<F>
            where
                F: FnMut(&str) -> Option<PathBuf>,
            {
                fn get(&mut self, source: &str) -> Result<Option<PathBuf>, String> {
                    Ok((self.0)(source))
                }
            }

            impl<F> asset_influx_import::Dependencies for Fn<F>
            where
                F: FnMut(&str, &str) -> Option<AssetId>,
            {
                fn get(&mut self, source: &str, target: &str) -> Result<Option<AssetId>, String> {
                    Ok((self.0)(source, target))
                }
            }

            let result = importer.import(
                &source_path,
                &output_path,
                &mut Fn(|src: &str| {
                    let src = item.source.join(src).ok()?; // If parsing fails - source will be listed in `ImportResult::RequireSources`.
                    let (path, modified) = sources.get(&src)?;
                    if let Some(modified) = modified {
                        item.sources.insert(src, modified);
                    }
                    Some(path.to_owned())
                }),
                &mut Fn(|src: &str, target: &str| {
                    let src = item.source.join(src).ok()?;

                    match SourceMeta::new(&src, base, external) {
                        Ok(meta) => {
                            let asset = meta.get_asset(target)?;
                            item.dependencies.insert(asset.id());
                            Some(asset.id())
                        }
                        Err(err) => {
                            tracing::error!("Fetching dependency failed. {:#}", err);
                            None
                        }
                    }
                }),
            );

            match result {
                Ok(()) => {}
                Err(ImportError::Other { reason }) => {
                    return Err(eyre::eyre!(
                        "Failed to import {}:{:?}->{}. {}",
                        item.source,
                        item.format,
                        item.target,
                        reason,
                    ))
                }
                Err(ImportError::RequireSources { sources: srcs }) => {
                    if item.attempt >= MAX_ITEM_ATTEMPTS {
                        return Err(eyre::eyre!(
                            "Failed to import {}:{:?}->{}. Too many attempts",
                            item.source,
                            item.format,
                            item.target,
                        ));
                    }

                    let source = item.source.clone();
                    for src in srcs {
                        match source.join(&src) {
                            Err(err) => {
                                return Err(eyre::eyre!(
                                    "Failed to join URL '{}' with '{}'. {:#}",
                                    source,
                                    src,
                                    err,
                                ))
                            }
                            Ok(url) => sources.fetch(&mut temporaries, &url).await?,
                        };
                    }
                    continue;
                }
                Err(ImportError::RequireDependencies { dependencies }) => {
                    if item.attempt >= MAX_ITEM_ATTEMPTS {
                        return Err(eyre::eyre!(
                            "Failed to import {}:{:?}->{}. Too many attempts",
                            item.source,
                            item.format,
                            item.target,
                        ));
                    }

                    let source = item.source.clone();
                    for dep in dependencies.into_iter() {
                        match source.join(&dep.source) {
                            Err(err) => {
                                return Err(eyre::eyre!(
                                    "Failed to join URL '{}' with '{}'. {:#}",
                                    source,
                                    dep.source,
                                    err,
                                ))
                            }
                            Ok(url) => {
                                stack.push(StackItem {
                                    source: url,
                                    format: None,
                                    target: dep.target,
                                    attempt: 0,
                                    sources: HashMap::new(),
                                    dependencies: HashSet::new(),
                                });
                            }
                        };
                    }
                    continue;
                }
            }

            if !artifacts.exists() {
                std::fs::create_dir_all(artifacts).wrap_err_with(|| {
                    format!(
                        "Failed to create artifacts directory '{}'",
                        artifacts.display()
                    )
                })?;

                if let Err(err) = std::fs::write(artifacts.join(".gitignore"), "*") {
                    tracing::error!(
                        "Failed to place .gitignore into artifacts directory. {:#}",
                        err
                    );
                }
            }

            let new_id = self.id_gen.new_id();

            let item = stack.pop().unwrap();

            let make_relative_source = |source| match self.base_url.make_relative(source) {
                None => item.source.to_string(),
                Some(source) => source,
            };

            let mut sources = Vec::new();
            if let Some(modified) = modified {
                sources.push((make_relative_source(&item.source), modified));
            }
            sources.extend(
                item.sources
                    .iter()
                    .map(|(url, modified)| (make_relative_source(url), *modified)),
            );

            let asset = AssetMeta::new(
                new_id,
                item.format.clone(),
                sources,
                item.dependencies.into_iter().collect(),
                &output_path,
                artifacts,
            )
            .wrap_err("Failed to prepare new asset")?;

            let artifact_path = asset.artifact_path(artifacts);

            meta.add_asset(item.target.clone(), asset, base, external)?;

            self.artifacts.write().insert(
                new_id,
                AssetItem {
                    source: item.source,
                    format: item.format,
                    target: item.target,
                },
            );

            if stack.is_empty() {
                return Ok((new_id, artifact_path));
            }
        }
    }

    /// Fetch asset data path.
    pub async fn fetch(&self, id: AssetId) -> Option<PathBuf> {
        let scanned = *self.scanned.read();

        if !scanned {
            let existing_artifacts: HashSet<_> = self.artifacts.read().keys().copied().collect();

            let mut new_artifacts = Vec::new();
            let mut scanned = self.scanned.write();

            if !*scanned {
                scan_local(&self.base, &existing_artifacts, &mut new_artifacts);
                scan_external(&self.external, &existing_artifacts, &mut new_artifacts);

                let mut artifacts = self.artifacts.write();
                for (id, item) in new_artifacts {
                    artifacts.insert(id, item);
                }

                *scanned = true;

                drop(artifacts);
                drop(scanned);
            }
        }

        let item = self.artifacts.read().get(&id).cloned()?;

        let (_, path) = self
            .store_url(item.source, item.format.as_deref(), &item.target)
            .await
            .ok()?;

        Some(path)
    }

    /// Fetch asset data path.
    pub async fn find_asset(
        &self,
        source: &str,
        target: &str,
    ) -> eyre::Result<Option<(AssetId, PathBuf)>> {
        let source_url = self.base_url.join(source).wrap_err_with(|| {
            format!(
                "Failed to construct URL from base '{}' and source '{}'",
                self.base_url, source
            )
        })?;

        let meta = SourceMeta::new(&source_url, &self.base, &self.external)
            .wrap_err("Failed to fetch source meta")?;

        match meta.get_asset(target) {
            None => {
                drop(meta);
                match self.store(source, None, target).await {
                    Err(err) => {
                        tracing::warn!(
                            "Failed to store '{}' as '{}' on lookup. {:#}",
                            source,
                            target,
                            err
                        );
                        Ok(None)
                    }
                    Ok(id) => Ok(Some(id)),
                }
            }
            Some(asset) => Ok(Some((
                asset.id(),
                asset.artifact_path(&self.artifacts_base),
            ))),
        }
    }
}

pub fn find_asset_influx_info(mut path: PathBuf) -> Option<PathBuf> {
    loop {
        path.push(ASSET_INFLUX_META_NAME);
        if path.is_file() {
            return Some(path);
        }
        path.pop();

        if !path.pop() {
            return None;
        }
    }
}

fn url_ext(url: &Url) -> Option<&str> {
    let path = url.path();
    let dot = path.rfind('.')?;
    let sep = path.rfind('/')?;
    if dot == path.len() || dot <= sep + 1 {
        None
    } else {
        Some(&path[dot + 1..])
    }
}

fn scan_external(
    external: &Path,
    existing_artifacts: &HashSet<AssetId>,
    artifacts: &mut Vec<(AssetId, AssetItem)>,
) {
    let dir = match std::fs::read_dir(&external) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!("External directory does not exists");
            return;
        }
        Err(err) => {
            tracing::error!(
                "Failed to scan directory '{}'. {:#}",
                external.display(),
                err
            );
            return;
        }
        Ok(dir) => dir,
    };
    for e in dir {
        let e = match e {
            Err(err) => {
                tracing::error!(
                    "Failed to read entry in directory '{}'. {:#}",
                    external.display(),
                    err,
                );
                return;
            }
            Ok(e) => e,
        };
        let name = e.file_name();
        let path = external.join(&name);
        let ft = match e.file_type() {
            Err(err) => {
                tracing::error!("Failed to check '{}'. {:#}", path.display(), err);
                continue;
            }
            Ok(ft) => ft,
        };
        if ft.is_file() && !SourceMeta::is_local_meta_path(&path) {
            let meta = match SourceMeta::open_external(&path) {
                Err(err) => {
                    tracing::error!("Failed to scan meta file '{}'. {:#}", path.display(), err);
                    continue;
                }
                Ok(meta) => meta,
            };

            let source = meta.url();

            for (target, asset) in meta.assets() {
                if !existing_artifacts.contains(&asset.id()) {
                    artifacts.push((
                        asset.id(),
                        AssetItem {
                            source: source.clone(),
                            format: asset.format().map(ToOwned::to_owned),
                            target: target.to_owned(),
                        },
                    ));
                }
            }
        }
    }
}

fn scan_local(
    base: &Path,
    existing_artifacts: &HashSet<AssetId>,
    artifacts: &mut Vec<(AssetId, AssetItem)>,
) {
    debug_assert!(base.is_absolute());

    if !base.exists() {
        tracing::info!("Local artifacts directory does not exists");
        return;
    }

    let mut queue = VecDeque::new();
    queue.push_back(base.to_owned());

    while let Some(dir_path) = queue.pop_front() {
        let dir = match std::fs::read_dir(&dir_path) {
            Err(err) => {
                tracing::error!(
                    "Failed to scan directory '{}'. {:#}",
                    dir_path.display(),
                    err
                );
                continue;
            }
            Ok(dir) => dir,
        };
        for e in dir {
            let e = match e {
                Err(err) => {
                    tracing::error!(
                        "Failed to read entry in directory '{}'. {:#}",
                        dir_path.display(),
                        err,
                    );
                    continue;
                }
                Ok(e) => e,
            };
            let name = e.file_name();
            let path = dir_path.join(&name);
            let ft = match e.file_type() {
                Err(err) => {
                    tracing::error!("Failed to check '{}'. {:#}", path.display(), err);
                    continue;
                }
                Ok(ft) => ft,
            };
            if ft.is_dir() {
                queue.push_back(path);
            } else if ft.is_file() && SourceMeta::is_local_meta_path(&path) {
                let meta = match SourceMeta::open_local(&path) {
                    Err(err) => {
                        tracing::error!("Failed to scan meta file '{}'. {:#}", path.display(), err);
                        continue;
                    }
                    Ok(meta) => meta,
                };

                let source = meta.url();
                for (target, asset) in meta.assets() {
                    if !existing_artifacts.contains(&asset.id()) {
                        artifacts.push((
                            asset.id(),
                            AssetItem {
                                source: source.clone(),
                                format: asset.format().map(ToOwned::to_owned),
                                target: target.to_owned(),
                            },
                        ));
                    }
                }
            }
        }
    }
}
