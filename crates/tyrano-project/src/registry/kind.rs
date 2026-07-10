//! The value-kind system for tag parameters.
//!
//! Scope discipline: this is **not** a general type system. It exists for
//! exactly two jobs — validating statically-written parameter values and
//! steering reference/asset resolution — and is complete *for that scope*:
//!
//! - [`ValueKind::Any`] is the top kind: every value satisfies it.
//! - Dynamic values (`&expr` entities, `%param` refs) satisfy **every**
//!   kind (gradual typing); they are never statically checked.
//! - There is no join/meet over kinds and deliberately no `Never`:
//!   checking only ever asks "does this given value satisfy this expected
//!   kind?", so no other operations are needed.

use crate::project::AssetKind;

/// What a parameter value is expected to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// Top: anything goes.
    Any,
    /// A `*label` reference (leading `*`, non-empty name).
    Label,
    /// A scenario-file reference (`storage=` on jump-family tags).
    /// Shape-checked as text here; existence is the cross-file checker's
    /// job.
    Scenario,
    /// An asset-file reference in the given namespace. Existence is the
    /// cross-file checker's job.
    Asset(AssetKind),
    /// An integer or float literal.
    Number,
    /// `true` or `false`.
    Boolean,
    /// `#rrggbb[aa]` or `0xrrggbb[aa]`.
    Color,
    /// A JavaScript expression — never statically validated.
    Expression,
    /// A game-variable path: `f.`/`sf.`/`tf.`/`mp.` plus segments.
    VariableName,
    /// One of a fixed word list.
    Enum(&'static [&'static str]),
    /// Free text.
    Text,
}
