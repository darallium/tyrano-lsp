//! Interned `storage=` strings.

/// A raw `storage=` value as written in a script (`"scene2.ks"`,
/// `"sub/ev.ks"`, `"scene2"`), interned so it can key tracked queries.
///
/// Deliberately *not* normalized: `"scene2"` and `"scene2.ks"` are
/// different names that happen to resolve to the same file — resolution
/// (with extension completion) is [`crate::resolve_storage`]'s job.
#[salsa::interned(no_lifetime, debug)]
pub struct StorageName {
    #[returns(ref)]
    pub text: String,
}
