use core::fmt;
use std::{
    any::{Any, TypeId},
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use ahash::RandomState;
use argosy_id::AssetId;
use hashbrown::hash_map::RawEntryMut;

use crate::{
    asset::{Asset, AssetBuild},
    error::{Error, NotFound},
    key::hash_id_key_erased,
    loader::{AssetShard, AssetState, DecodedState, PathShard, PathState},
};

#[derive(Clone)]
pub(crate) enum State {
    Searching {
        key_hash: u64,
        path_shard: PathShard,
        asset_shards: Arc<[AssetShard]>,
        random_state: RandomState,
    },
    Loading {
        key_hash: u64,
        shard: AssetShard,
    },
    Loaded {
        key_hash: u64,
        shard: AssetShard,
    },
    Ready {
        asset: Arc<dyn Any + Send + Sync>,
    },
    Error {
        error: Error,
    },
    Missing,
}

/// Internal implementation of asset handle types.
#[derive(Clone)]
pub struct Handle {
    pub(crate) type_id: TypeId,
    pub(crate) id: Option<AssetId>,
    pub(crate) path: Option<Arc<str>>,
    pub(crate) state: State,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PollFor {
    Id,
    Load,
    Ready,
}

impl Handle {
    #[inline]
    fn id(&self) -> Result<AssetId, Error> {
        if let Some(id) = self.id {
            return Ok(id);
        }
        match &self.state {
            State::Missing => Err(Error::new(NotFound {
                id: None,
                path: self.path.clone(),
            })),
            State::Error { error } => Err(error.clone()),
            _ => unreachable!(),
        }
    }

    /// Polls asset handle for loading progress.
    fn poll(&mut self, poll_for: PollFor, waker: Option<&Waker>) -> bool {
        match &mut self.state {
            State::Searching {
                key_hash,
                path_shard,
                asset_shards,
                random_state,
            } => {
                let path = self
                    .path
                    .as_deref()
                    .expect("This state is only reachable when asset is requested with path");

                let mut locked_shard = path_shard.lock();
                let raw_entry = locked_shard
                    .raw_entry_mut()
                    .from_hash(*key_hash, |k| k.eq_key_erased(self.type_id, path));

                match raw_entry {
                    RawEntryMut::Vacant(_) => {
                        panic!("This state is only reachable when asset is requested with path")
                    }
                    RawEntryMut::Occupied(mut entry) => match entry.get_mut() {
                        PathState::Unloaded {
                            id_wakers,
                            asset_wakers,
                        } => {
                            match poll_for {
                                PollFor::Id | PollFor::Load => {
                                    waker.map(|waker| id_wakers.push(waker.clone()));
                                }
                                PollFor::Ready => {
                                    waker.map(|waker| asset_wakers.push(waker.clone()));
                                }
                            }
                            return false;
                        }
                        PathState::Loaded { id } => {
                            let id = *id;
                            drop(locked_shard);
                            self.id = Some(id);

                            let key_hash = hash_id_key_erased(self.type_id, id, &*random_state);

                            let shard =
                                asset_shards[key_hash as usize % asset_shards.len()].clone();

                            self.state = State::Loading { key_hash, shard };
                            if poll_for == PollFor::Id {
                                return true;
                            }
                        }
                        PathState::Missing => {
                            drop(locked_shard);
                            self.state = State::Missing;
                            return true;
                        }
                    },
                }
            }
            _ => {
                debug_assert!(self.id.is_some());

                if poll_for == PollFor::Id {
                    return true;
                }
            }
        }

        match &mut self.state {
            State::Searching { .. } => unreachable!(),
            State::Loaded { .. } if poll_for != PollFor::Ready => {
                return true;
            }
            State::Loading { key_hash, shard } | State::Loaded { key_hash, shard } => {
                let id = self
                    .id
                    .expect("This state can be reached only with known id");
                let mut locked_shard = shard.lock();
                let raw_entry = locked_shard
                    .raw_entry_mut()
                    .from_hash(*key_hash, |k| k.eq_key_erased(self.type_id, id));

                match raw_entry {
                    RawEntryMut::Vacant(_) => {
                        unreachable!("AssetResult existence guarantee entry is not vacant")
                    }
                    RawEntryMut::Occupied(mut entry) => match entry.get_mut() {
                        AssetState::Unloaded { wakers } => {
                            waker.map(|waker| wakers.push(waker.clone()));
                            false
                        }
                        AssetState::Loaded { wakers, .. } if poll_for == PollFor::Ready => {
                            waker.map(|waker| wakers.push(waker.clone()));
                            drop(locked_shard);
                            self.state = State::Loaded {
                                key_hash: *key_hash,
                                shard: shard.clone(),
                            };
                            false
                        }
                        AssetState::Loaded { .. } => {
                            drop(locked_shard);
                            self.state = State::Loaded {
                                key_hash: *key_hash,
                                shard: shard.clone(),
                            };
                            true
                        }
                        AssetState::Ready { .. } => {
                            drop(locked_shard);
                            self.state = State::Loaded {
                                key_hash: *key_hash,
                                shard: shard.clone(),
                            };
                            true
                        }
                        AssetState::Missing => {
                            drop(locked_shard);
                            self.state = State::Missing;
                            return true;
                        }
                        AssetState::Error { error } => {
                            let error = error.clone();
                            drop(locked_shard);
                            self.state = State::Error { error };
                            return true;
                        }
                    },
                }
            }
            _ => true,
        }
    }

    /// Builds loaded asset if not yet built.
    /// Uses appropriate closure to make result value.
    /// If asset is built `get` is called.
    /// If asset is missing `missing` is called.
    /// If asset load or build failed `err` is called.
    ///
    /// # Panics
    ///
    /// This function may panic if called before `poll(PollFor::Load)` returned `true`.
    fn build<F, G, M, E, R>(&mut self, build_fn: F, get: G, missing: M, err: E) -> R
    where
        F: FnOnce(
            &mut (dyn Any + Send + Sync),
        ) -> Option<Result<Arc<dyn Any + Send + Sync>, Error>>,
        G: FnOnce(&Arc<dyn Any + Send + Sync>) -> R,
        M: FnOnce(Option<AssetId>, Option<&Arc<str>>) -> R,
        E: FnOnce(&Error) -> R,
    {
        match &mut self.state {
            State::Searching { .. } | State::Loading { .. } => {
                unreachable!("`poll_load` must be used first")
            }
            State::Loaded { key_hash, shard } => {
                let id = self
                    .id
                    .expect("This state can be reached only with known id");

                let mut locked_shard = shard.lock();
                let raw_entry = locked_shard
                    .raw_entry_mut()
                    .from_hash(*key_hash, |k| k.eq_key_erased(self.type_id, id));

                match raw_entry {
                    RawEntryMut::Vacant(_) => {
                        unreachable!("AssetResult existence guarantee entry is not vacant")
                    }
                    RawEntryMut::Occupied(mut entry) => match entry.get_mut() {
                        AssetState::Unloaded { .. } => {
                            unreachable!("`poll_load` must be used first")
                        }
                        AssetState::Ready { asset, .. } => {
                            let result = get(asset);
                            drop(locked_shard);
                            result
                        }
                        AssetState::Loaded { decoded, .. } => {
                            let decode = decoded.clone();
                            drop(locked_shard);

                            let mut lock = decode.lock();
                            let opt = build_fn(&mut *lock);

                            let mut locked_shard = shard.lock();
                            drop(lock);

                            let raw_entry = locked_shard
                                .raw_entry_mut()
                                .from_hash(*key_hash, |k| k.eq_key_erased(self.type_id, id));

                            match raw_entry {
                                RawEntryMut::Vacant(_) => unreachable!(),
                                RawEntryMut::Occupied(mut entry) => match entry.get_mut() {
                                    AssetState::Unloaded { .. } | AssetState::Missing => {
                                        unreachable!()
                                    }
                                    AssetState::Error { error } => err(&error),
                                    AssetState::Ready { asset, .. } => get(asset),
                                    AssetState::Loaded {
                                        source, version, ..
                                    } => match opt {
                                        None => unreachable!(),
                                        Some(result) => match result {
                                            Ok(asset) => {
                                                let out = get(&asset);
                                                *entry.get_mut() = AssetState::Ready {
                                                    asset,
                                                    source: *source,
                                                    version: *version,
                                                };
                                                out
                                            }
                                            Err(error) => {
                                                let out = err(&error);
                                                *entry.get_mut() = AssetState::Error { error };
                                                out
                                            }
                                        },
                                    },
                                },
                            }
                        }
                        AssetState::Missing => {
                            drop(locked_shard);
                            self.state = State::Missing;
                            missing(self.id, self.path.as_ref())
                        }
                        AssetState::Error { error } => {
                            let error = error.clone();
                            drop(locked_shard);
                            let result = err(&error);
                            self.state = State::Error { error };
                            result
                        }
                    },
                }
            }
            State::Ready { asset } => get(asset),
            State::Missing => missing(self.id, self.path.as_ref()),
            State::Error { error } => err(error),
        }
    }

    /// If asset is loaded and built `get` is called.
    /// If asset is missing `missing` is called.
    /// If asset load or build failed `err` is called.
    ///
    /// # Panics
    ///
    /// This function may panic if called before `poll(PollFor::Build)` returned `true`.
    fn get<G, M, E, R>(&mut self, get: G, missing: M, err: E) -> R
    where
        G: FnOnce(&Arc<dyn Any + Send + Sync>) -> R,
        M: FnOnce(Option<AssetId>, Option<&Arc<str>>) -> R,
        E: FnOnce(&Error) -> R,
    {
        match &mut self.state {
            State::Searching { .. } | State::Loading { .. } => {
                unreachable!("`poll_load(..)` must be used first")
            }
            State::Loaded { key_hash, shard } => {
                let id = self
                    .id
                    .expect("This state can be reached only with known id");
                let mut locked_shard = shard.lock();
                let raw_entry = locked_shard
                    .raw_entry_mut()
                    .from_hash(*key_hash, |k| k.eq_key_erased(self.type_id, id));

                match raw_entry {
                    RawEntryMut::Vacant(_) => {
                        unreachable!("AssetResult existence guarantee entry is not vacant")
                    }
                    RawEntryMut::Occupied(mut entry) => match entry.get_mut() {
                        AssetState::Unloaded { .. } => {
                            unreachable!("`poll(..)` must be used first")
                        }
                        AssetState::Loaded { .. } => {
                            unreachable!("`poll(true, ..)` must be used first")
                        }
                        AssetState::Ready { asset, .. } => {
                            let result = get(asset);
                            drop(locked_shard);
                            result
                        }
                        AssetState::Missing => {
                            drop(locked_shard);
                            self.state = State::Missing;
                            missing(self.id, self.path.as_ref())
                        }
                        AssetState::Error { error } => {
                            let error = error.clone();
                            drop(locked_shard);
                            let result = err(&error);
                            self.state = State::Error { error };
                            result
                        }
                    },
                }
            }
            State::Ready { asset } => get(asset),
            State::Missing => missing(self.id, self.path.as_ref()),
            State::Error { error } => err(error),
        }
    }
}

/// Handle returned from `Loader::load` or `Loader::load_with_id`.
/// The asset may be in any state.
/// Another state can be polled using polling methods or awaiting on futures
/// returned from `AssetHandle::id()`, `AssetHandle::loaded()`, `AssetHandle::built()`.
///
/// The handle can be awaited directly, which is equivalent to `AssetHandle::loaded()`.
///
/// It is also possible to erase asset type and replace it with specific builder type
/// using `AssetHandle::driver()`. This way drivers for any asset types that share
/// the same builder type can be stored in the same collection and
/// polled together.
#[derive(Clone)]
pub struct AssetHandle<A> {
    /// If asset is already loaded and built this field contains it.
    result: Option<Result<A, Error>>,

    /// Internal handle implementation.
    handle: Handle,
}

impl<A> Unpin for AssetHandle<A> {}

impl<A> fmt::Debug for AssetHandle<A>
where
    A: Asset,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.handle.id, &self.handle.path) {
            (None, None) => unreachable!(),
            (_, Some(path)) => {
                write!(f, "{}({})", A::name(), path)
            }
            (Some(id), _) => {
                write!(f, "{}({})", A::name(), id)
            }
        }
    }
}

impl<A> PartialEq for AssetHandle<A> {
    fn eq(&self, other: &Self) -> bool {
        match (self.handle.id, other.handle.id) {
            (Some(id1), Some(id2)) => return id1 == id2,
            _ => {}
        }
        match (self.handle.path.as_deref(), other.handle.path.as_deref()) {
            (Some(path1), Some(path2)) => return path1 == path2,
            _ => {}
        }

        // It maybe refer to the same asset, but one handle is fetched with id
        // while another is fetched with path and id is not yet known.
        false
    }
}

impl<A> AssetHandle<A> {
    pub(crate) fn new(handle: Handle) -> Self {
        AssetHandle {
            result: None,
            handle,
        }
    }
}

impl<A> AssetHandle<A> {
    /// Returns a future to wait for asset loaded via path to be identified.
    /// Resolves to asset id or error.
    #[inline]
    pub fn id(self) -> AssetLookup {
        AssetLookup {
            handle: self.handle,
        }
    }

    /// Polls for asset loaded via path to be identified.
    /// Returns some result with asset or error.
    /// Returns none if asset is not yet identified.
    #[inline]
    pub fn poll_id(&mut self) -> Option<Result<AssetId, Error>> {
        if let Some(id) = self.handle.id {
            return Some(Ok(id));
        }

        if !self.handle.poll(PollFor::Id, None) {
            return None;
        }

        Some(self.handle.id())
    }
}

/// Future to wait for asset loaded via path to be identified.
pub struct AssetLookup {
    handle: Handle,
}

impl Future for AssetLookup {
    type Output = Result<AssetId, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let me = self.get_mut();
        if let Some(id) = me.handle.id {
            return Poll::Ready(Ok(id));
        }

        if !me.handle.poll(PollFor::Id, Some(cx.waker())) {
            return Poll::Pending;
        }

        Poll::Ready(me.handle.id())
    }
}

impl<A> AssetHandle<A>
where
    A: Clone + 'static,
{
    /// Returns a future to wait for asset to be ready.
    /// Resolves to asset or error.
    #[inline]
    pub fn ready(self) -> AssetFuture<A> {
        AssetFuture {
            result: self.result,
            handle: self.handle,
        }
    }

    /// Polls for asset to be ready.
    /// Returns some result with asset or error.
    /// Returns none if asset is not yet ready.
    #[inline]
    pub fn poll_ready(&mut self) -> Option<Result<A, Error>> {
        if let Some(result) = self.result.clone() {
            return Some(result);
        }

        if !self.handle.poll(PollFor::Ready, None) {
            return None;
        }

        let result = self.handle.get(
            |asset| {
                let asset = asset.downcast_ref::<A>().unwrap();
                Ok(asset.clone())
            },
            |id, path| {
                Err(Error::new(NotFound {
                    path: path.cloned(),
                    id,
                }))
            },
            |err| Err(err.clone()),
        );

        self.result = Some(result.clone());
        Some(result)
    }
}

/// Future to wait for asset to be ready.
pub struct AssetFuture<A> {
    result: Option<Result<A, Error>>,
    handle: Handle,
}

impl<A> Unpin for AssetFuture<A> {}

impl<A> Future for AssetFuture<A>
where
    A: Clone + 'static,
{
    type Output = Result<A, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<A, Error>> {
        let me = self.get_mut();

        if let Some(result) = me.result.clone() {
            return Poll::Ready(result);
        }

        if !me.handle.poll(PollFor::Ready, Some(cx.waker())) {
            return Poll::Pending;
        }

        let result = me.handle.get(
            |asset| {
                let asset = asset.downcast_ref::<A>().unwrap();
                Ok(asset.clone())
            },
            |id, path| {
                Err(Error::new(NotFound {
                    path: path.cloned(),
                    id,
                }))
            },
            |err| Err(err.clone()),
        );

        me.result = Some(result.clone());
        Poll::Ready(result)
    }
}

impl<A> AssetHandle<A>
where
    A: Clone,
{
    /// Returns a future to wait for asset to be loaded.
    /// Resolves to loaded asset handle that can be used to build asset or error.
    #[inline]
    pub fn loaded(self) -> Self {
        self
    }

    /// Polls for asset to be loaded.
    /// Returns some result with loaded asset handle that can be used to build asset or error.
    /// Returns none if asset is not yet loaded.
    #[inline]
    pub fn poll_loaded(&mut self) -> Option<Result<LoadedAsset<A>, Error>> {
        if let Some(result) = self.result.clone() {
            return Some(result.map(|asset| LoadedAsset {
                result: Some(Ok(asset.clone())),
                handle: self.handle.clone(),
            }));
        }

        if !self.handle.poll(PollFor::Load, None) {
            return None;
        }

        match &self.handle.state {
            State::Error { error } => Some(Err(error.clone())),
            State::Missing => Some(Err(Error::new(NotFound {
                id: self.handle.id.clone(),
                path: self.handle.path.clone(),
            }))),
            State::Searching { .. } => unreachable!(),
            _ => Some(Ok(LoadedAsset {
                result: None,
                handle: self.handle.clone(),
            })),
        }
    }

    /// Polls for asset and builds it if loaded.
    /// Returns some result with asset or error.
    /// Returns none if asset is not yet loaded.
    #[inline]
    pub fn poll_build<B>(&mut self, builder: &mut B) -> Option<Result<A, Error>>
    where
        A: AssetBuild<B>,
    {
        if let Some(result) = self.result.clone() {
            return Some(result);
        }

        if !self.handle.poll(PollFor::Load, None) {
            return None;
        }

        let result = self.handle.build(
            move |decoded| {
                let decoded = decoded.downcast_mut::<DecodedState<A>>().unwrap().take()?;

                match A::build(builder, decoded) {
                    Ok(asset) => Some(Ok(Arc::new(asset.clone()))),
                    Err(err) => {
                        let err = Error::new(err);
                        Some(Err(err.clone()))
                    }
                }
            },
            |asset| {
                let asset = asset.downcast_ref::<A>().unwrap();
                Ok(asset.clone())
            },
            |id, path| {
                Err(Error::new(NotFound {
                    path: path.cloned(),
                    id,
                }))
            },
            |err| Err(err.clone()),
        );

        self.result = Some(result.clone());
        Some(result)
    }
}

impl<A> Future for AssetHandle<A> {
    type Output = Result<LoadedAsset<A>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<LoadedAsset<A>, Error>> {
        let me = self.get_mut();
        if !me.handle.poll(PollFor::Load, Some(cx.waker())) {
            return Poll::Pending;
        }

        match &me.handle.state {
            State::Error { error } => Poll::Ready(Err(error.clone())),
            State::Missing => Poll::Ready(Err(Error::new(NotFound {
                id: me.handle.id.clone(),
                path: me.handle.path.clone(),
            }))),
            State::Searching { .. } => unreachable!(),
            _ => Poll::Ready(Ok(LoadedAsset {
                result: None,
                handle: me.handle.clone(),
            })),
        }
    }
}

/// Handle returned by awaiting on `AssetHandle::loaded()`.
/// The asset is loaded and can be built.
pub struct LoadedAsset<A> {
    /// If asset is already loaded and built this field contains it.
    result: Option<Result<A, Error>>,

    handle: Handle,
}

impl<A> LoadedAsset<A>
where
    A: Asset,
{
    /// Build loaded asset.
    /// Returns result with asset or error.
    pub fn build<B>(&mut self, builder: &mut B) -> Result<A, Error>
    where
        A: AssetBuild<B>,
    {
        if let Some(result) = &self.result {
            match result {
                Ok(asset) => return Ok(asset.clone()),
                Err(error) => return Err(error.clone()),
            }
        }

        self.handle.build(
            move |decoded| {
                let decoded = decoded.downcast_mut::<DecodedState<A>>().unwrap().take()?;

                match A::build(builder, decoded) {
                    Ok(asset) => Some(Ok(Arc::new(asset.clone()))),
                    Err(err) => {
                        let err = Error::new(err);
                        Some(Err(err.clone()))
                    }
                }
            },
            |asset| {
                let asset = asset.downcast_ref::<A>().unwrap();
                Ok(asset.clone())
            },
            |id, path| {
                Err(Error::new(NotFound {
                    path: path.cloned(),
                    id,
                }))
            },
            |err| Err(err.clone()),
        )
    }
}

pub trait DriveAsset {
    type Builder<'a>;
}

pub enum SimpleDrive<B> {
    #[doc(hidden)]
    _Unused(B),
}

impl<B> DriveAsset for SimpleDrive<B> {
    type Builder<'a> = B;
}

pub enum NoBuilderDrive {}

impl DriveAsset for NoBuilderDrive {
    type Builder<'a> = ();
}

impl<A> AssetHandle<A>
where
    A: Asset,
{
    /// Returns a future to wait for asset to be loaded
    /// erasing asset type but providing specific builder type.
    #[inline]
    pub fn driver<D>(self) -> AssetDriver<D>
    where
        D: DriveAsset,
        A: for<'a> AssetBuild<D::Builder<'a>>,
    {
        AssetDriver {
            handle: self.handle,
            build_fn: build_fn::<A, D>,
        }
    }
}

/// Future to wait for asset to be loaded.
/// Unlike `AssetHandle` it is
/// parametrized with builder type instead of asset type.
pub struct AssetDriver<D: DriveAsset = NoBuilderDrive> {
    handle: Handle,
    build_fn: fn(
        decoded: &mut (dyn Any + Send + Sync),
        builder: &mut D::Builder<'_>,
    ) -> Option<Result<Arc<dyn Any + Send + Sync>, Error>>,
}

impl<D> AssetDriver<D>
where
    D: DriveAsset,
{
    /// Polls for asset to be loaded.
    /// Returns `true` if asset is loaded.
    /// Returns `false` if asset is not yet loaded.
    #[inline]
    pub fn poll_loaded(&mut self) -> Option<LoadedAssetDriver<D>> {
        if !self.handle.poll(PollFor::Load, None) {
            return None;
        }

        Some(LoadedAssetDriver {
            handle: self.handle.clone(),
            build_fn: self.build_fn,
        })
    }

    /// Polls for asset and builds it if loaded.
    /// Returns `true` if asset is loaded and built.
    /// Returns `false` if asset is not yet loaded.
    #[inline]
    pub fn poll_build(&mut self, builder: &mut D::Builder<'_>) -> bool {
        if !self.handle.poll(PollFor::Load, None) {
            return false;
        }

        self.handle.build(
            |decoded| (self.build_fn)(decoded, builder),
            |_| {},
            |_, _| {},
            |_| {},
        );
        true
    }
}

impl<D> Future for AssetDriver<D>
where
    D: DriveAsset,
{
    type Output = LoadedAssetDriver<D>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<LoadedAssetDriver<D>> {
        let me = self.get_mut();
        if !me.handle.poll(PollFor::Load, Some(cx.waker())) {
            return Poll::Pending;
        }

        Poll::Ready(LoadedAssetDriver {
            handle: me.handle.clone(),
            build_fn: me.build_fn,
        })
    }
}

/// Handle returned by awaiting on `AssetDriver`.
/// The asset is loaded and can be built.
/// Unlike `LoadedAsset` it is
/// parametrized with builder type instead of asset type.
pub struct LoadedAssetDriver<D: DriveAsset = NoBuilderDrive> {
    handle: Handle,
    build_fn: fn(
        decoded: &mut (dyn Any + Send + Sync),
        builder: &mut D::Builder<'_>,
    ) -> Option<Result<Arc<dyn Any + Send + Sync>, Error>>,
}

impl<D> LoadedAssetDriver<D>
where
    D: DriveAsset,
{
    #[inline]
    pub fn build(mut self, builder: &mut D::Builder<'_>) {
        self.handle.build(
            |decoded| (self.build_fn)(decoded, builder),
            |_| {},
            |_, _| {},
            |_| {},
        )
    }
}

fn build_fn<A, D>(
    decoded: &mut (dyn Any + Send + Sync),
    builder: &mut D::Builder<'_>,
) -> Option<Result<Arc<dyn Any + Send + Sync>, Error>>
where
    D: DriveAsset,
    A: for<'a> AssetBuild<D::Builder<'a>>,
{
    let decoded = decoded.downcast_mut::<DecodedState<A>>().unwrap().take()?;

    match A::build(builder, decoded) {
        Ok(asset) => Some(Ok(Arc::new(asset.clone()))),
        Err(err) => {
            let err = Error::new(err);
            Some(Err(err.clone()))
        }
    }
}
