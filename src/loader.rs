use std::{
    any::{Any, TypeId},
    hash::{BuildHasher, Hasher},
    sync::Arc,
    task::Waker,
};

use ahash::RandomState;
use argosy_id::AssetId;
use hashbrown::hash_map::{HashMap, RawEntryMut};
use parking_lot::Mutex;
use smallvec::SmallVec;
use tracing::Instrument;

use crate::{
    error::Error,
    handle::{AssetHandle, Handle, State},
    key::{hash_path_key, PathKey},
};

use crate::{
    asset::Asset,
    key::{hash_id_key, Key, TypeKey},
    source::Source,
};

/// This is default number of shards per CPU for shared hash map of asset states.
const DEFAULT_SHARDS_PER_CPU: usize = 8;

struct Data {
    bytes: Box<[u8]>,
    version: u64,
    source: usize,
}

/// Builder for [`Loader`].
/// Allows configure asset loader with required [`Source`]s.
pub struct LoaderBuilder {
    num_shards: usize,
    sources: Vec<Box<dyn Source>>,
}

impl Default for LoaderBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl LoaderBuilder {
    /// Returns new [`LoaderBuilder`] without asset sources.
    pub fn new() -> Self {
        let num_cpus = num_cpus::get();
        let num_shards = DEFAULT_SHARDS_PER_CPU * num_cpus;

        LoaderBuilder {
            num_shards,
            sources: Vec::new(),
        }
    }

    /// Adds provided source to the loader.
    pub fn add(&mut self, source: impl Source) -> &mut Self {
        self.sources.push(Box::new(source));
        self
    }

    /// Adds provided source to the loader.
    pub fn with(mut self, source: impl Source) -> Self {
        self.sources.push(Box::new(source));
        self
    }

    /// Adds provided source to the loader.
    pub fn add_dyn(&mut self, source: Box<dyn Source>) -> &mut Self {
        self.sources.push(source);
        self
    }

    /// Adds provided source to the loader.
    pub fn wit_dyn(mut self, source: Box<dyn Source>) -> Self {
        self.sources.push(source);
        self
    }

    /// Sets number of shards for the loader.
    ///
    /// Actual number of shards will be bumped to the next power of two
    /// and limited to 512.
    ///
    /// This is low-level optimization tweaking function.
    /// Default value should be sufficient most use cases.
    pub fn set_num_shards(&mut self, num_shards: usize) -> &mut Self {
        self.num_shards = num_shards;
        self
    }

    /// Sets number of shards for the loader.
    ///
    /// Actual number of shards will be bumped to the next power of two
    /// and limited to 512.
    ///
    /// This is low-level optimization tweaking function.
    /// Default value should be sufficient most use cases.
    pub fn with_num_shards(mut self, num_shards: usize) -> Self {
        self.num_shards = num_shards;
        self
    }

    /// Builds and returns new [`Loader`] instance.
    pub fn build(self) -> Loader {
        let random_state = RandomState::new();
        let sources: Arc<[_]> = self.sources.into();

        let asset_shards: Vec<AssetShard> = (0..self.num_shards)
            .map(|_| Arc::new(Mutex::new(HashMap::with_hasher(random_state.clone()))))
            .collect();

        let path_shards: Vec<PathShard> = (0..self.num_shards)
            .map(|_| Arc::new(Mutex::new(HashMap::with_hasher(random_state.clone()))))
            .collect();

        Loader {
            sources,
            random_state,
            asset_cache: asset_shards.into(),
            path_cache: path_shards.into(),
        }
    }
}

pub(crate) type AssetShard = Arc<Mutex<HashMap<TypeKey, AssetState, RandomState>>>;
pub(crate) type PathShard = Arc<Mutex<HashMap<PathKey, PathState, RandomState>>>;

/// Virtual storage for all available assets.
#[derive(Clone)]
pub struct Loader {
    /// Array of available asset sources.
    sources: Arc<[Box<dyn Source>]>,

    /// Hasher to pick a shard.
    random_state: RandomState,

    /// Cache with asset states.
    asset_cache: Arc<[AssetShard]>,

    /// Cache with path states.
    path_cache: Arc<[PathShard]>,
}

pub(crate) type DecodedState<A> = Option<<A as Asset>::Decoded>;

pub(crate) enum AssetState {
    /// Not yet loaded asset.
    Unloaded {
        wakers: WakeOnDrop,
    },
    Loaded {
        // Contains `DecodedState<A>`
        decoded: Arc<spin::Mutex<dyn Any + Send + Sync>>,
        version: u64,
        source: usize,
        wakers: WakeOnDrop,
    },
    Ready {
        // Contains `A`
        asset: Arc<dyn Any + Send + Sync>,
        version: u64,
        source: usize,
    },
    /// All sources reported that asset is missing.
    Missing,
    Error {
        error: Error,
    },
}

pub(crate) enum PathState {
    /// Not yet loaded asset.
    Unloaded {
        asset_wakers: WakeOnDrop,
        id_wakers: WakeOnDrop,
    },

    /// Asset is loaded. Lookup main entry by this id.
    Loaded { id: AssetId },

    /// All sources reported that asset is missing.
    Missing,
}

impl Loader {
    /// Returns [`LoaderBuilder`] instance
    pub fn builder() -> LoaderBuilder {
        LoaderBuilder::new()
    }

    pub fn load_with_id<A: Asset>(&self, id: AssetId) -> AssetHandle<A> {
        // Hash asset key.
        let key_hash = hash_id_key::<A>(id, &self.random_state);

        // Use asset key hash to pick a shard.
        // It will always pick same shard for same key.
        let shards_len = self.asset_cache.len();
        let shard = &self.asset_cache[key_hash as usize % shards_len];

        // Lock picked shard.
        let mut locked_shard = shard.lock();

        // Find an entry into sharded hashmap.
        let asset_entry = locked_shard
            .raw_entry_mut()
            .from_hash(key_hash, |k| k.eq_key::<A>(id));

        match asset_entry {
            RawEntryMut::Occupied(entry) => {
                // Already queried. See status.
                match entry.get() {
                    AssetState::Unloaded { .. } => AssetHandle::new(Handle {
                        type_id: TypeId::of::<A>(),
                        path: None,
                        id: Some(id),
                        state: State::Loading {
                            key_hash,
                            shard: shard.clone(),
                        },
                    }),
                    AssetState::Error { error } => AssetHandle::new(Handle {
                        type_id: TypeId::of::<A>(),
                        path: None,
                        id: Some(id),
                        state: State::Error {
                            error: error.clone(),
                        },
                    }),
                    AssetState::Missing => AssetHandle::new(Handle {
                        type_id: TypeId::of::<A>(),
                        path: None,
                        id: Some(id),
                        state: State::Missing,
                    }),
                    AssetState::Loaded { .. } => AssetHandle::new(Handle {
                        type_id: TypeId::of::<A>(),
                        path: None,
                        id: Some(id),
                        state: State::Loaded {
                            key_hash,
                            shard: shard.clone(),
                        },
                    }),
                    AssetState::Ready { asset, .. } => AssetHandle::new(Handle {
                        type_id: TypeId::of::<A>(),
                        path: None,
                        id: Some(id),
                        state: State::Ready {
                            asset: asset.clone(),
                        },
                    }),
                }
            }
            RawEntryMut::Vacant(entry) => {
                let asset_key = TypeKey::new::<A>(id);

                // Register query
                let _ = entry.insert_hashed_nocheck(
                    key_hash,
                    asset_key,
                    AssetState::Unloaded {
                        wakers: WakeOnDrop::new(),
                    },
                );
                drop(locked_shard);

                let shard = shard.clone();

                let handle = AssetHandle::new(Handle {
                    type_id: TypeId::of::<A>(),
                    path: None,
                    id: Some(id),
                    state: State::Loading {
                        key_hash,
                        shard: shard.clone(),
                    },
                });

                let loader = self.clone();
                tokio::spawn(
                    async move {
                        load_asset_task::<A>(&loader, shard, key_hash, id).await;
                    }
                    .in_current_span(),
                );

                handle
            }
        }
    }

    /// Load asset with specified key (path or id) and returns handle
    /// that can be used to access assets once it is loaded.
    ///
    /// If asset was previously requested it will not be re-loaded,
    /// but handle to shared state will be returned instead,
    /// even if first load was not successful or different format was used.
    pub fn load<'a, A, K>(&self, key: K) -> AssetHandle<A>
    where
        A: Asset,
        K: Into<Key<'a>>,
    {
        match key.into() {
            Key::Path(path) => {
                // Hash asset path key.
                let mut hasher = self.random_state.build_hasher();
                hash_path_key::<A, _>(path, &mut hasher);
                let key_hash = hasher.finish();

                // Use asset key hash to pick a shard.
                // It will always pick same shard for same key.
                let shards_len = self.path_cache.len();
                let path_shard = &self.path_cache[key_hash as usize % shards_len];

                // Lock picked shard.
                let mut locked_shard = path_shard.lock();

                // Find an entry into sharded hashmap.
                let raw_entry = locked_shard
                    .raw_entry_mut()
                    .from_hash(key_hash, |k| k.eq_key::<A>(path));

                match raw_entry {
                    RawEntryMut::Occupied(entry) => {
                        // Already queried. See status.

                        let path_key = entry.key().clone();
                        match entry.get() {
                            PathState::Unloaded { .. } => {
                                drop(locked_shard);

                                AssetHandle::new(Handle {
                                    type_id: TypeId::of::<A>(),
                                    path: Some(path_key.path),
                                    id: None,
                                    state: State::Searching {
                                        key_hash,
                                        path_shard: path_shard.clone(),
                                        asset_shards: self.asset_cache.clone(),
                                        random_state: self.random_state.clone(),
                                    },
                                })
                            }
                            PathState::Loaded { id } => {
                                let id = *id;
                                drop(locked_shard);

                                self.load_with_id(id)
                            }
                            PathState::Missing => AssetHandle::new(Handle {
                                type_id: TypeId::of::<A>(),
                                path: Some(path_key.path.clone()),
                                id: None,
                                state: State::Missing,
                            }),
                        }
                    }
                    RawEntryMut::Vacant(entry) => {
                        let path_key = PathKey::new::<A>(path.into());
                        let path = path_key.path.clone();

                        // Register query
                        let _ = entry.insert_hashed_nocheck(
                            key_hash,
                            path_key.clone(),
                            PathState::Unloaded {
                                asset_wakers: WakeOnDrop::new(),
                                id_wakers: WakeOnDrop::new(),
                            },
                        );
                        drop(locked_shard);

                        let path_shard = path_shard.clone();

                        let handle = AssetHandle::new(Handle {
                            type_id: TypeId::of::<A>(),
                            path: Some(path_key.path),
                            id: None,
                            state: State::Searching {
                                key_hash,
                                path_shard: path_shard.clone(),
                                asset_shards: self.asset_cache.clone(),
                                random_state: self.random_state.clone(),
                            },
                        });

                        let loader = self.clone();
                        tokio::spawn(
                            async move {
                                find_asset_task::<A>(&loader, path_shard, key_hash, &path).await;
                            }
                            .in_current_span(),
                        );

                        handle
                    }
                }
            }
            Key::Id(id) => self.load_with_id(id),
        }
    }
}

async fn load_asset_task<A: Asset>(loader: &Loader, shard: AssetShard, key_hash: u64, id: AssetId) {
    let new_state = match load_asset(&loader.sources, id).await {
        Err(error) => AssetState::Error { error },
        Ok(None) => AssetState::Missing,
        Ok(Some(data)) => {
            let result = A::decode(data.bytes, loader).await;

            match result {
                Err(err) => AssetState::Error {
                    error: Error::new(err),
                },
                Ok(decoded) => AssetState::Loaded {
                    decoded: Arc::new(spin::Mutex::new(Some(decoded))),
                    version: data.version,
                    source: data.source,
                    wakers: WakeOnDrop::new(),
                },
            }
        }
    };

    // Change state and notify waters.
    let mut locked_shard = shard.lock();

    let entry = locked_shard
        .raw_entry_mut()
        .from_hash(key_hash, |k| k.eq_key::<A>(id));

    match entry {
        RawEntryMut::Vacant(_) => {
            unreachable!("No other code could change the state")
        }
        RawEntryMut::Occupied(mut entry) => {
            let entry = entry.get_mut();
            match entry {
                AssetState::Unloaded { .. } => {
                    *entry = new_state;
                }
                _ => unreachable!("No other code could change the state"),
            }
        }
    }
}

// Task to find asset using path.
async fn find_asset_task<A: Asset>(
    loader: &Loader,
    path_shard: PathShard,
    key_hash: u64,
    path: &str,
) {
    let opt = find_asset::<A>(&loader.sources, path).await;
    match opt {
        None => {
            // Asset not found. Change state and notify waters.
            let mut locked_shard = path_shard.lock();

            let entry = locked_shard
                .raw_entry_mut()
                .from_hash(key_hash, |k| k.eq_key::<A>(path));

            match entry {
                RawEntryMut::Vacant(_) => {
                    unreachable!("No other code could change the state")
                }
                RawEntryMut::Occupied(mut entry) => {
                    let entry = entry.get_mut();
                    match entry {
                        PathState::Unloaded { .. } => {
                            *entry = PathState::Missing;
                        }
                        _ => unreachable!("No other code could change the state"),
                    }
                }
            }
        }
        Some(id) => {
            // Asset found. Change the state

            let asset_shard;
            let asset_key_hash;
            {
                // Taking wakers from path state
                // and either moving them to asset state
                // or waking them.
                let mut moving_wakers = WakeOnDrop::new();

                let mut locked_shard = path_shard.lock();

                let entry = locked_shard
                    .raw_entry_mut()
                    .from_hash(key_hash, |k| k.eq_key::<A>(path));

                match entry {
                    RawEntryMut::Vacant(_) => {
                        unreachable!("No other code could change the state")
                    }
                    RawEntryMut::Occupied(mut entry) => {
                        let state = entry.get_mut();
                        match state {
                            PathState::Unloaded { asset_wakers, .. } => {
                                // Decide what to do with asset wakers later.
                                moving_wakers.append(&mut asset_wakers.vec);
                                *state = PathState::Loaded { id };
                            }
                            _ => unreachable!("No other code could change the state"),
                        }
                    }
                }

                // Hash asset key.
                asset_key_hash = hash_id_key::<A>(id, &loader.random_state);

                // Check ID entry.
                let shard_idx = asset_key_hash as usize % loader.asset_cache.len();
                asset_shard = loader.asset_cache[shard_idx].clone();

                let mut locked_shard = asset_shard.lock();

                let entry = locked_shard
                    .raw_entry_mut()
                    .from_hash(asset_key_hash, |k| k.eq_key::<A>(id));

                match entry {
                    RawEntryMut::Vacant(entry) => {
                        // Asset was not requested by ID yet.
                        let asset_key = TypeKey::new::<A>(id);

                        // Register query
                        let _ = entry.insert_hashed_nocheck(
                            asset_key_hash,
                            asset_key,
                            AssetState::Unloaded {
                                wakers: moving_wakers,
                            }, // Put wakers here.
                        );
                    }
                    RawEntryMut::Occupied(mut entry) => {
                        match entry.get_mut() {
                            AssetState::Unloaded { wakers } => {
                                // Move wakers to ID entry.
                                wakers.append(&mut moving_wakers.vec);
                            }
                            _ => {
                                // Loading is complete one way or another.
                                // Wake wakers from path entry.
                            }
                        }
                        return;
                    }
                }
            }

            // Proceed loading by ID.
            load_asset_task::<A>(loader, asset_shard, asset_key_hash, id).await;
        }
    }
}

async fn load_asset(sources: &[Box<dyn Source>], id: AssetId) -> Result<Option<Data>, Error> {
    for (index, source) in sources.iter().enumerate() {
        if let Some(asset) = source.load(id).await? {
            return Ok(Some(Data {
                bytes: asset.bytes,
                version: asset.version,
                source: index,
            }));
        }
    }
    Ok(None)
}

async fn find_asset<A: Asset>(sources: &[Box<dyn Source>], path: &str) -> Option<AssetId> {
    for source in sources {
        if let Some(id) = source.find(path, A::name()).await {
            return Some(id);
        }
    }
    None
}

type WakersVec = SmallVec<[Waker; 4]>;

// Convenient type to wake wakers on scope exit.
pub(crate) struct WakeOnDrop {
    vec: WakersVec,
}

impl WakeOnDrop {
    pub fn new() -> Self {
        WakeOnDrop {
            vec: WakersVec::new(),
        }
    }

    pub fn append(&mut self, v: &mut WakersVec) {
        self.vec.append(v);
    }

    pub fn push(&mut self, waker: Waker) {
        self.vec.push(waker);
    }
}

impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        for waker in self.vec.drain(..) {
            waker.wake()
        }
    }
}
