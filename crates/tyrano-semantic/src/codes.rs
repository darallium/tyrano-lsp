//! Stable diagnostic codes for the cross-file (`xsem-`) checkers.
//!
//! `&'static str` constants matching `tyrano_db::FileDiagnostic::code`;
//! prefix `xsem-` distinguishes this layer from the file-local `sem-`
//! codes of `tyrano-parser-core`.

/// A tag that is neither a builtin nor any visible macro (warning).
pub const UNKNOWN_TAG: &str = "xsem-unknown-tag";
/// A builtin tag missing one of its required parameters (error).
pub const MISSING_PARAM: &str = "xsem-missing-param";
/// A parameter a `Deny`-schema builtin does not declare (warning).
pub const UNKNOWN_PARAM: &str = "xsem-unknown-param";
/// A static parameter value violating its declared kind (error).
pub const KIND_MISMATCH: &str = "xsem-kind-mismatch";
/// `storage=` naming no existing scenario file (error).
pub const UNKNOWN_STORAGE: &str = "xsem-unknown-storage";
/// A `target=*label` that the (existing) storage file lacks (error).
pub const UNKNOWN_LABEL_IN_STORAGE: &str = "xsem-unknown-label-in-storage";
/// An asset reference no asset root provides (warning).
pub const MISSING_ASSET: &str = "xsem-missing-asset";
