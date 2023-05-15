use std::{
    any::TypeId,
    fmt,
    hash::{BuildHasher, Hash, Hasher},
    sync::Arc,
};

use argosy_id::AssetId;

use crate::asset::Asset;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct TypeKey {
    pub type_id: TypeId,
    pub id: AssetId,
}

impl TypeKey {
    #[inline(always)]
    pub fn new<A: Asset>(asset: AssetId) -> Self {
        TypeKey {
            type_id: TypeId::of::<A>(),
            id: asset,
        }
    }

    #[inline(always)]
    pub fn eq_key<A: Asset>(&self, asset: AssetId) -> bool {
        self.type_id == TypeId::of::<A>() && self.id == asset
    }

    #[inline(always)]
    pub fn eq_key_erased(&self, type_id: TypeId, asset: AssetId) -> bool {
        self.type_id == type_id && self.id == asset
    }
}

#[inline(always)]
pub fn hash_id_key<A>(id: AssetId, state: &impl BuildHasher) -> u64
where
    A: Asset,
{
    let mut hasher = state.build_hasher();
    TypeId::of::<A>().hash(&mut hasher);
    id.hash(&mut hasher);
    hasher.finish()
}

#[inline(always)]
pub fn hash_id_key_erased(type_id: TypeId, id: AssetId, state: &impl BuildHasher) -> u64 {
    let mut hasher = state.build_hasher();
    type_id.hash(&mut hasher);
    id.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PathKey {
    pub type_id: TypeId,
    pub path: Arc<str>,
}

impl PathKey {
    #[inline(always)]
    pub fn new<A: Asset>(asset: Arc<str>) -> Self {
        PathKey {
            type_id: TypeId::of::<A>(),
            path: asset,
        }
    }

    #[inline(always)]
    pub fn eq_key<A: Asset>(&self, asset: &str) -> bool {
        self.type_id == TypeId::of::<A>() && *self.path == *asset
    }

    #[inline(always)]
    pub fn eq_key_erased(&self, type_id: TypeId, asset: &str) -> bool {
        self.type_id == type_id && *self.path == *asset
    }
}

pub fn hash_path_key<A, H>(path: &str, state: &mut H)
where
    A: Asset,
    H: Hasher,
{
    TypeId::of::<A>().hash(state);
    path.hash(state);
}

#[derive(Clone, Copy)]
pub enum Key<'a> {
    Path(&'a str),
    Id(AssetId),
}

impl fmt::Debug for Key<'_> {
    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Path(path) => fmt::Debug::fmt(path, f),
            Key::Id(id) => fmt::Debug::fmt(id, f),
        }
    }
}

impl fmt::Display for Key<'_> {
    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Path(path) => fmt::Display::fmt(path, f),
            Key::Id(id) => fmt::Display::fmt(id, f),
        }
    }
}

impl<'a, S> From<&'a S> for Key<'a>
where
    S: AsRef<str> + ?Sized,
{
    #[inline(always)]
    fn from(s: &'a S) -> Self {
        Key::Path(s.as_ref())
    }
}

impl From<AssetId> for Key<'_> {
    #[inline(always)]
    fn from(id: AssetId) -> Self {
        Key::Id(id)
    }
}
