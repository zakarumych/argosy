//! Argosy is asset management library.
//! It can be used to load assets from different sources, like files, network, etc.
//!
//! It supports asynchronous loading, caching, dependency trees (and someday hot-reloading).
//!
//! The entry point is [`Loader`] type that can be built with a number of [`Source`]s.
//! Assets are primarily identified by [`AssetId`] and secondarily by string key.
//! If [`AssetId`] is not known
//! One of the built-in sources is `FileSource` that loads assets from files in a directory.
//!
//! Argosy provides derive macro to turn structures into assets that
//! can depend on other assets.
//!
//! # Asset and AssetField derive macros
//!
//! Creates structures to act as two loading stages of asset and implement asset using those.
//! First stages must be deserializable with serde.
//! All fields with `#[external]` must implement `AssetField<External>`. Which has blanket impl for `Asset` implementors and some wrappers, like `Option<A>` and `Arc<[A]>` where `A: Asset`.
//! All fields without special attributes must implement `AssetField<Inlined>`.
//! Types that implement `DeserializeOwned` automatically implement `AssetField<Inlined>`.
//! It can be derived using `derive(AssetField)`. They can in turn contain fields with `#[external]` attributes. Also implemented for wrappers like `Option<A>` and `Arc<[A]>`.
//! All fields transiently with `#[external]` attribute will be decoded as `AssetId` and then loaded recursively.
//!
//! # Example
//!
//! ```
//! # use argosy::*;
//! /// Simple deserializable type. Included as-is into generated types for `#[derive(Asset)]` and #[derive(AssetField)].
//! #[derive(Clone, serde::Deserialize)]
//! struct Foo;
//!
//! /// Trivial asset type.
//! #[derive(Clone, Asset)]
//! struct Bar;
//!
//! /// Asset field type. `AssetField<Container>` implementation is generated, but not `Asset` implementation.
//! /// Fields of types with `#[derive(AssetField)]` attribute are not replaced by uuids as external assets.
//! #[derive(Clone, AssetField)]
//! struct Baz;
//!
//! /// Asset structure. Implements Asset trait using
//! /// two generated structures are intermediate phases.
//! #[derive(Clone, Asset)]
//! #[asset(name = "MyAssetStruct")]
//! struct AssetStruct {
//!     /// Deserializable types are inlined into asset as is.
//!     foo: Foo,
//!
//!     /// `AssetField<External>` is implemented for all `Asset` implementors.
//!     /// Deserialized as `AssetId` and loaded recursively.
//!     #[asset(external)]
//!     bar: Bar,
//!
//!     /// Container fields are deserialized similar to types that derive `Asset`.
//!     /// If there is no external asset somewhere in hierarchy, decoded `Baz` is structurally equivalent to `Baz`.
//!     baz: Baz,
//! }
//! ```

mod asset;
mod error;
mod field;
mod handle;
mod key;
mod loader;
mod source;

pub use self::{
    asset::{Asset, AssetBuild, LeafAsset, TrivialAsset},
    error::{Error, NotFound},
    field::{AssetField, AssetFieldBuild},
    handle::{
        AssetDriver, AssetFuture, AssetHandle, AssetLookup, DriveAsset, LoadedAsset,
        LoadedAssetDriver, SimpleDrive,
    },
    key::Key,
    loader::{Loader, LoaderBuilder},
    source::{AssetData, Source},
};

pub use argosy_id::AssetId;

pub use argosy_proc::{self as proc, Asset, AssetField};

/// Error type used by derive-macro.
#[derive(::std::fmt::Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("Failed to deserialize asset info from json")]
    Json(#[source] serde_json::Error),

    #[error("Failed to deserialize asset info from bincode")]
    Bincode(#[source] bincode::Error),
}

#[doc(hidden)]
pub mod proc_macro {
    pub use std::{
        boxed::Box,
        convert::{From, Infallible},
        fmt::Debug,
        future::{ready, Ready},
        result::Result::{self, Err, Ok},
    };

    pub use futures::future::BoxFuture;
    pub use serde::{Deserialize, Serialize};
    use serde_json::error::Category;
    pub use thiserror::Error;

    pub use crate::{
        asset::{Asset, AssetBuild, TrivialAsset},
        field::{AssetField, AssetFieldBuild, External, FieldBuilder, Inlined},
        loader::Loader,
        DecodeError,
    };

    #[inline(never)]
    pub fn deserialize_info<T: serde::de::DeserializeOwned>(
        bytes: &[u8],
    ) -> Result<T, DecodeError> {
        if bytes.is_empty() {
            // Zero-length is definitely bincode.
            match bincode::deserialize(&*bytes) {
                Ok(value) => Ok(value),
                Err(err) => Err(DecodeError::Bincode(err)),
            }
        } else {
            match serde_json::from_slice(&*bytes) {
                Ok(value) => Ok(value),
                Err(err) => match err.classify() {
                    Category::Syntax => {
                        // That's not json. Bincode then.
                        match bincode::deserialize(&*bytes) {
                            Ok(value) => Ok(value),
                            Err(err) => Err(DecodeError::Bincode(err)),
                        }
                    }
                    _ => Err(DecodeError::Json(err)),
                },
            }
        }
    }
}
