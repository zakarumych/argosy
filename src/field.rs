use std::{
    convert::Infallible,
    future::{ready, Future, Ready},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use argosy_id::AssetId;
use futures::future::TryJoinAll;

use crate::{
    asset::{Asset, AssetBuild},
    error::Error,
    handle::{AssetHandle, LoadedAsset},
    loader::Loader,
};

pub struct FieldBuilder<'a, B>(pub &'a mut B);

#[doc(hidden)]
pub enum External {}

#[doc(hidden)]
pub enum Inlined {}

/// This trait can be derived for types to allow using them as asset fields.
///
/// It is auto-implemented for all types that implement `serde::de::DeserializeOwned`.
/// As well as `Option<A>` where `A: AssetField` and `Arc<[A]>` where `A: AssetField`.
pub trait AssetField<K = Inlined>: Clone + Sized + Send + Sync + 'static {
    /// Deserializable data.
    type Info: serde::de::DeserializeOwned;

    /// Decoded representation of this asset.
    type Decoded: Send + Sync;

    /// Decoding error.
    type DecodeError: std::error::Error + Send + Sync + 'static;

    /// Building error.
    type BuildError: std::error::Error + Send + Sync + 'static;

    /// Future that will resolve into decoded asset when ready.
    type Fut: Future<Output = Result<Self::Decoded, Self::DecodeError>> + Send;

    fn decode(info: Self::Info, loader: &Loader) -> Self::Fut;
}

/// Builder trait for asset fields.
///
/// It is auto-implemented for all types that implement `serde::de::DeserializeOwned`.
pub trait AssetFieldBuild<K, A: AssetField<K>> {
    /// Build asset instance using decoded representation and `Resources`.
    fn build(self, decoded: A::Decoded) -> Result<A, A::BuildError>;
}

impl<A> AssetField<External> for Option<A>
where
    A: AssetField<External>,
{
    type Info = Option<A::Info>;
    type Decoded = Option<A::Decoded>;
    type DecodeError = A::DecodeError;
    type BuildError = A::BuildError;
    type Fut = MaybeFuture<A::Fut>;

    #[inline]
    fn decode(info: Option<A::Info>, loader: &Loader) -> Self::Fut {
        match info {
            None => MaybeFuture(None),
            Some(info) => MaybeFuture(Some(A::decode(info, loader))),
        }
    }
}

impl<B, A> AssetFieldBuild<External, Option<A>> for FieldBuilder<'_, B>
where
    A: AssetField<External>,
    for<'a> FieldBuilder<'a, B>: AssetFieldBuild<External, A>,
{
    #[inline]
    fn build(self, maybe_decoded: Option<A::Decoded>) -> Result<Option<A>, A::BuildError> {
        match maybe_decoded {
            Some(decoded) => self.build(decoded).map(Some),
            None => Ok(None),
        }
    }
}

pub struct MaybeFuture<F>(Option<F>);

impl<F, R, E> Future for MaybeFuture<F>
where
    F: Future<Output = Result<R, E>>,
{
    type Output = Result<Option<R>, E>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let maybe_fut = unsafe { self.map_unchecked_mut(|me| &mut me.0) }.as_pin_mut();

        match maybe_fut {
            None => Poll::Ready(Ok(None)),
            Some(fut) => match fut.poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(result) => Poll::Ready(result.map(Some)),
            },
        }
    }
}

impl<A> AssetField<External> for Arc<[A]>
where
    A: AssetField<External>,
{
    type Info = Vec<A::Info>;
    type Decoded = Vec<A::Decoded>;
    type DecodeError = A::DecodeError;
    type BuildError = A::BuildError;
    type Fut = TryJoinAll<A::Fut>;

    #[inline]
    fn decode(info: Vec<A::Info>, loader: &Loader) -> Self::Fut {
        info.into_iter()
            .map(|info| A::decode(info, loader))
            .collect()
    }
}

impl<B, A> AssetFieldBuild<External, Arc<[A]>> for FieldBuilder<'_, B>
where
    A: AssetField<External>,
    for<'a> FieldBuilder<'a, B>: AssetFieldBuild<External, A>,
{
    #[inline]
    fn build(self, decoded: Vec<A::Decoded>) -> Result<Arc<[A]>, A::BuildError> {
        decoded
            .into_iter()
            .map(move |decoded| FieldBuilder(self.0).build(decoded))
            .collect()
    }
}

impl<A> AssetField<External> for A
where
    A: Asset,
{
    type Info = AssetId;
    type Decoded = LoadedAsset<A>;
    type DecodeError = Error;
    type BuildError = Error;
    type Fut = AssetHandle<A>;

    #[inline(never)]
    fn decode(id: AssetId, loader: &Loader) -> Self::Fut {
        loader.load(id)
    }
}

impl<B, A> AssetFieldBuild<External, A> for FieldBuilder<'_, B>
where
    A: Asset,
    A: AssetBuild<B>,
{
    #[inline(never)]
    fn build(self, mut ready: LoadedAsset<A>) -> Result<A, Error> {
        ready.build(self.0)
    }
}

impl<T> AssetField<Inlined> for T
where
    T: serde::de::DeserializeOwned + Clone + Sized + Send + Sync + 'static,
{
    type Info = T;
    type Decoded = T;
    type DecodeError = Infallible;
    type BuildError = Infallible;
    type Fut = Ready<Result<T, Infallible>>;

    #[inline(never)]
    fn decode(value: T, _: &Loader) -> Self::Fut {
        ready(Ok(value))
    }
}

impl<B, T> AssetFieldBuild<Inlined, T> for FieldBuilder<'_, B>
where
    T: serde::de::DeserializeOwned + Clone + Sized + Send + Sync + 'static,
{
    #[inline(never)]
    fn build(self, decoded: T) -> Result<T, Infallible> {
        Ok(decoded)
    }
}
