use std::{fmt, sync::Arc};

use argosy_id::AssetId;

use crate::asset::Asset;

/// Error value that is returned from fallible methods when asset is missing.
#[derive(thiserror::Error)]
pub struct NotFound {
    /// Path that was used to search for the asset.
    /// `None` if asset was requested by [`AssetId`].
    pub path: Option<Arc<str>>,

    /// Asset identifier.
    /// `None` if asset was requested by path and identifier was not found.
    pub id: Option<AssetId>,
}

impl fmt::Display for NotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.path, &self.id) {
            (None, None) => f.write_str(
                "Failed to load an asset. [No AssetId or path provided - this is a bug].",
            ),
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

/// Error that can be returned from methods of handlers.
/// This type wraps any error that can occur during asset loading and building.
///
/// It can be downcast to the original error type using [`Error::downcast_ref`].
///
/// If asset is missing and API returns either asset or [`Error`], the error
/// would contain [`NotFound`] error.
///
/// If asset loading failed, the error would contain error of the [`Source`] that
/// failed to load the asset.
///
/// If asset decoding failed, the error would contain [`A::DecodeError`].
/// If asset building failed, the error would contain [`A::BuildError`].
#[derive(Clone)]
#[repr(transparent)]
pub struct Error(Arc<dyn std::error::Error + Send + Sync>);

impl Error {
    /// Creates a new [`Error`] from any error type.
    pub fn new<E>(error: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Error(Arc::new(error))
    }

    /// Checks if this error is of given type.
    #[inline]
    pub fn is<E: std::error::Error + 'static>(&self) -> bool {
        self.0.is::<E>()
    }

    /// Checks if this error is [`NotFound`].
    #[inline]
    pub fn is_not_found(&self) -> bool {
        self.0.is::<NotFound>()
    }

    /// Checks if this error is [`DecodeError`] for given asset type.
    #[inline]
    pub fn is_decode_error<A: Asset>(&self) -> bool {
        self.0.is::<A::DecodeError>()
    }

    /// Checks if this error is [`BuildError`] for given asset type.
    #[inline]
    pub fn is_build_error<A: Asset>(&self) -> bool {
        self.0.is::<A::BuildError>()
    }

    /// Downcasts this error to the original error type if guessed correctly.
    #[inline]
    pub fn downcast_ref<E: std::error::Error + 'static>(&self) -> Option<&E> {
        self.0.downcast_ref()
    }

    /// Downcasts this error to [`NotFound`] if it is [`NotFound`].
    #[inline]
    pub fn get_not_found(&self) -> Option<&NotFound> {
        self.0.downcast_ref()
    }

    /// Downcasts this error to [`DecodeError`] for given asset type if it is [`DecodeError`].
    #[inline]
    pub fn get_decode_error<A: Asset>(&self) -> Option<&A::DecodeError> {
        self.0.downcast_ref()
    }

    /// Downcasts this error to [`BuildError`] for given asset type if it is [`BuildError`].
    #[inline]
    pub fn get_build_error<A: Asset>(&self) -> Option<&A::BuildError> {
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
