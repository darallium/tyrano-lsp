//! The builtin-tag registry: *data only*.
//!
//! This module describes what tags exist and what their parameters mean
//! ([`TagSpec`], [`ParamSpec`], [`ValueKind`]); the checking rules that
//! consume these descriptions live in `tyrano-semantic` (`check/`). The
//! registry is static data, so it is a [`std::sync::LazyLock`], not a
//! salsa query.

mod builtin;
mod kind;
mod schema;

pub use builtin::BUILTIN_TAGS;
pub use kind::ValueKind;
pub use schema::{ExtraParams, GLOBAL_PARAMS, ParamSpec, TagRegistry, TagSpec, builtin_registry};
