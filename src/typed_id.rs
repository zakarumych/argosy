use std::{
    borrow::Borrow,
    fmt::{self, Debug, Display, LowerHex, UpperHex},
    marker::PhantomData,
};

use argosy_id::AssetId;

use crate::Asset;

/// `AssetId` augmented with type information, specifying which asset type is referenced.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct TypedAssetId<A> {
    pub id: AssetId,
    pub marker: PhantomData<A>,
}

impl<A> TypedAssetId<A> {
    pub const fn new(value: u64) -> Option<Self> {
        match AssetId::new(value) {
            None => None,
            Some(id) => Some(TypedAssetId {
                id,
                marker: PhantomData,
            }),
        }
    }
}

impl<A> Borrow<AssetId> for TypedAssetId<A> {
    fn borrow(&self) -> &AssetId {
        &self.id
    }
}

impl<A> Debug for TypedAssetId<A>
where
    A: Asset,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            write!(f, "{}({:#?})", A::name(), self.id)
        } else {
            write!(f, "{}({:?})", A::name(), self.id)
        }
    }
}

impl<A> Display for TypedAssetId<A>
where
    A: Asset,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            write!(f, "{}({:#})", A::name(), self.id)
        } else {
            write!(f, "{}({:})", A::name(), self.id)
        }
    }
}

impl<A> LowerHex for TypedAssetId<A>
where
    A: Asset,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            write!(f, "{}({:#x})", A::name(), self.id)
        } else {
            write!(f, "{}({:x})", A::name(), self.id)
        }
    }
}

impl<A> UpperHex for TypedAssetId<A>
where
    A: Asset,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            write!(f, "{}({:#X})", A::name(), self.id)
        } else {
            write!(f, "{}({:X})", A::name(), self.id)
        }
    }
}
