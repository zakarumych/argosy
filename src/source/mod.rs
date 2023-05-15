pub mod fs;

use argosy_id::AssetId;
use futures::{future::BoxFuture, TryFutureExt};

use crate::error::Error;

/// Asset data loaded from [`Source`].
pub struct AssetData {
    /// Serialized asset data.
    pub bytes: Box<[u8]>,

    /// Opaque version for asset.
    /// It can only by interpreted by [`Source`]
    /// that returned this [`AssetData`] instance.
    pub version: u64,
}

/// Abstract source for asset raw data.
pub trait Source: Send + Sync + 'static {
    /// Error that may occur during asset loading.
    type Error: std::error::Error + Send + Sync;

    /// Searches for the asset by given path.
    /// Returns `Ok(Some(asset_data))` if asset is found and loaded successfully.
    /// Returns `Ok(None)` if asset is not found.
    fn find<'a>(&'a self, path: &'a str, asset: &'a str) -> BoxFuture<'a, Option<AssetId>>;

    /// Load asset data from this source.
    /// Returns `Ok(Some(asset_data))` if asset is loaded successfully.
    /// Returns `Ok(None)` if asset is not found, allowing checking other sources.
    fn load<'a>(&'a self, id: AssetId) -> BoxFuture<'a, Result<Option<AssetData>, Self::Error>>;

    /// Update asset data if newer is available.
    fn update<'a>(
        &'a self,
        id: AssetId,
        version: u64,
    ) -> BoxFuture<'a, Result<Option<AssetData>, Self::Error>>;
}

pub(crate) trait AnySource: Send + Sync + 'static {
    fn find<'a>(&'a self, path: &'a str, asset: &'a str) -> BoxFuture<'a, Option<AssetId>>;
    fn load<'a>(&'a self, id: AssetId) -> BoxFuture<'a, Result<Option<AssetData>, Error>>;
    fn update<'a>(
        &'a self,
        id: AssetId,
        version: u64,
    ) -> BoxFuture<'a, Result<Option<AssetData>, Error>>;
}

impl<S> AnySource for S
where
    S: Source,
{
    fn find<'a>(&'a self, path: &'a str, asset: &'a str) -> BoxFuture<'a, Option<AssetId>> {
        let fut = Source::find(self, path, asset);
        Box::pin(fut)
    }

    fn load<'a>(&'a self, id: AssetId) -> BoxFuture<'a, Result<Option<AssetData>, Error>> {
        let fut = Source::load(self, id);
        Box::pin(fut.map_err(Error::new))
    }

    fn update<'a>(
        &'a self,
        id: AssetId,
        version: u64,
    ) -> BoxFuture<'a, Result<Option<AssetData>, Error>> {
        let fut = Source::update(self, id, version);
        Box::pin(fut.map_err(Error::new))
    }
}
