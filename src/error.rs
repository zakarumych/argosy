use std::{fmt, sync::Arc};

use argosy_id::AssetId;

#[derive(thiserror::Error)]
pub struct NotFound {
    pub path: Option<Arc<str>>,
    pub id: Option<AssetId>,
}

impl fmt::Display for NotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.path, &self.id) {
            (None, None) => f.write_str("Failed to load an asset. [No AssetId or path provided]"),
            (Some(path), None) => write!(f, "Failed to load asset '{}'", path),
            (None, Some(id)) => write!(f, "Failed to load asset '{}'", id),
            (Some(path), Some(id)) => write!(f, "Failed to load asset '{} @ {}'", id, path),
        }
    }
}

impl fmt::Debug for NotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Error that can be returned from methods of [`Loader`] and handlers.
/// This type wraps any error that can occur during asset loading and building.
/// It can be downcast to the original error type using [`Error::downcast_ref`].
///
/// If asset is missing and API returns either asset or [`Error`], the error
/// would contain [`NotFound`] error.
///
/// If asset loading failed, the error would contain error of the [`Source`] that
/// failed to load the asset.
///
/// If asset decoding failed, the error would contain [`A::DecodeError`].
///
/// If asset building failed, the error would contain [`A::BuildError`].
#[derive(Clone)]
#[repr(transparent)]
pub struct Error(Arc<dyn std::error::Error + Send + Sync>);

impl Error {
    pub fn new<E>(error: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Error(Arc::new(error))
    }

    pub fn downcast_ref<E: std::error::Error + 'static>(&self) -> Option<&E> {
        self.0.downcast_ref()
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.0, f)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}
