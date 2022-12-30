//! Asset loader.
//!
//! # Asset and AssetField derive macros
//!
//! Creates structures to act as two loading stages of asset and implement asset using those.
//! First stages must be deserializable with serde.
//! All fields with `#[external]` must implement `AssetField<External>`. Which has blanket impl for `Asset` implementors and some wrappers, like `Option<A>` and `Arc<[A]>` where `A: Asset`.
//! All fields with `#[container]` attribute must implement `AssetField<Container>`. It can be derived using `derive(AssetField)`. They can in turn contain fields with `#[external]` and `#[container]` attributes. Also implemented for wrappers like `Option<A>` and `Arc<[A]>`.
//! All fields without special attributes of the target struct must implement `DeserializeOwned`.
//! All fields transiently with #[external] attribute will be replaced with id for first stage struct and `AssetResult`s for second stage.
//! Second stages will have `AssetResult`s fields in place of the assets.
//!
//! # Example
//!
//! ```
//!
//! # use goods::*;
//!
//! /// Simple deserializable type. Included as-is into generated types for `#[derive(Asset)]` and #[derive(AssetField)].
//! #[derive(Clone, serde::Deserialize)]
//! struct Foo;
//!
//! /// Trivial asset type.
//! #[derive(Clone, Asset)]
//! #[asset(name = "bar")]
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
//! #[asset(name = "assetstruct")]
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
//!     #[asset(container)]
//!     baz: Baz,
//! }
//! ```

mod asset;
mod field;
mod key;
mod loader;
pub mod source;
mod typed_id;

pub use self::{
    asset::{Asset, AssetBuild, SimpleAsset, TrivialAsset},
    field::{AssetField, AssetFieldBuild, Container, External},
    loader::{
        AssetHandle, AssetLookup, AssetResult, AssetResultPoisoned, Error, Key, Loader,
        LoaderBuilder, NotFound,
    },
};
pub use asset_influx_proc::{Asset, AssetField};

// Used by generated code.
#[doc(hidden)]
pub use {bincode, serde, serde_json, std::convert::Infallible, thiserror};

/// Error type used by derive-macro.
#[derive(::std::fmt::Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("Failed to deserialize asset info from json")]
    Json(#[source] serde_json::Error),

    #[error("Failed to deserialize asset info from bincode")]
    Bincode(#[source] bincode::Error),
}
