use std::{
    convert::Infallible,
    future::{ready, Ready},
};

use {
    crate::loader::Loader,
    std::{error::Error, future::Future},
};

/// An asset type that can be built from decoded representation.
pub trait Asset: Clone + Sized + Send + Sync + 'static {
    /// Decoded representation of this asset.
    ///
    /// Value of this type is the result of parsing raw bytes and optionally
    /// using `loader` to requested sub-assets and await them.
    type Decoded: Send + Sync;

    /// Decoding error.
    type DecodeError: Error + Send + Sync + 'static;

    /// Building error.
    type BuildError: Error + Send + Sync + 'static;

    /// Future that will resolve into decoded asset when ready.
    type Fut: Future<Output = Result<Self::Decoded, Self::DecodeError>> + Send;

    /// Asset name.
    fn name() -> &'static str;

    /// Decode asset from bytes loaded from asset source.
    fn decode(bytes: Box<[u8]>, loader: &Loader) -> Self::Fut;
}

/// Asset building trait.
///
/// There should be at least on implementation of this trait for each `Asset` type.
pub trait AssetBuild<B>: Asset {
    /// Build asset instance using decoded representation.
    fn build(builder: &mut B, decoded: Self::Decoded) -> Result<Self, Self::BuildError>;
}

/// Leaf assets have no dependencies.
/// For this reason their `decode` function is always sync and do not take `Loader` argument.
pub trait LeafAsset: Clone + Sized + Send + Sync + 'static {
    /// Decoded representation of this asset.
    ///
    /// Value of this type is the result of parsing raw bytes and optionally
    /// using `loader` to requested sub-assets and await them.
    type Decoded: Send + Sync;

    /// Decoding error.
    type DecodeError: Error + Send + Sync + 'static;

    /// Building error.
    type BuildError: Error + Send + Sync + 'static;

    /// Asset name.
    fn name() -> &'static str;

    /// Decode asset from bytes loaded from asset source.
    fn decode(bytes: Box<[u8]>) -> Result<Self::Decoded, Self::DecodeError>;
}

/// Trivial assets have no dependencies and do not require building.
/// They are decoded directly from bytes.
/// And thus any type implements `AssetBuilder<Self>`.
pub trait TrivialAsset: Clone + Sized + Send + Sync + 'static {
    type Error: Error + Send + Sync + 'static;

    /// Asset name.
    fn name() -> &'static str;

    /// Decode asset directly.
    fn decode(bytes: Box<[u8]>) -> Result<Self, Self::Error>;
}

impl<A> Asset for A
where
    A: LeafAsset,
{
    type Decoded = A::Decoded;
    type DecodeError = A::DecodeError;
    type BuildError = Infallible;
    type Fut = Ready<Result<A::Decoded, A::DecodeError>>;

    /// Asset name.
    #[inline]
    fn name() -> &'static str {
        <A as LeafAsset>::name()
    }

    #[inline]
    fn decode(bytes: Box<[u8]>, _: &Loader) -> Ready<Result<A::Decoded, A::DecodeError>> {
        ready(<A as LeafAsset>::decode(bytes))
    }
}

impl<A> LeafAsset for A
where
    A: TrivialAsset,
{
    type Decoded = A;
    type DecodeError = A::Error;
    type BuildError = Infallible;

    /// Asset name.
    #[inline]
    fn name() -> &'static str {
        <A as TrivialAsset>::name()
    }

    #[inline]
    fn decode(bytes: Box<[u8]>) -> Result<A, A::Error> {
        TrivialAsset::decode(bytes)
    }
}

impl<A, B> AssetBuild<B> for A
where
    A: TrivialAsset,
{
    #[inline(never)]
    fn build(_: &mut B, decoded: A) -> Result<A, Infallible> {
        Ok(decoded)
    }
}
