//! The builtin TyranoScript tag table.
//!
//! One category module per slice of [`BUILTIN_TAG_GROUPS`]; together they
//! cover every tag defined in the reference engine's
//! `tyrano/plugins/kag/kag.tag*.js` (TyranoScript master). Per tag, the
//! engine's `vital` array became the required parameters and its `pm`
//! object became the declared parameter list (with the engine defaults
//! where they are meaningful scalars).
//!
//! Kind-mapping conventions, applied uniformly across the modules:
//!
//! - `storage` follows the tag family's asset namespace (`playbgm` →
//!   `Asset(Bgm)`, `chara_*` → `Asset(FgImage)`, jump-family → `Scenario`,
//!   …). Where the engine's directory mapping is ambiguous the parameter
//!   stays [`ValueKind::Text`] so no false missing-asset warning fires.
//! - `target` is a [`ValueKind::Label`]; `exp`/`preexp`/`cond` are
//!   [`ValueKind::Expression`]s.
//! - Parameters with numeric engine defaults or coordinate/duration names
//!   are [`ValueKind::Number`]; `"true"`/`"false"` defaults are
//!   [`ValueKind::Boolean`]; colors are [`ValueKind::Color`]; fixed word
//!   lists evident in the engine source are [`ValueKind::Enum`]s.
//! - [`ExtraParams::Deny`] only for tags whose parameter set is closed
//!   (control flow, waits); visual tags with styling tails and tags that
//!   consume arbitrary parameters (`anim`, keyframe CSS) say
//!   [`ExtraParams::Allow`].

mod anim;
mod audio;
mod camera;
mod chara;
mod flow;
mod layer;
mod message;
mod system;
mod three;
mod vchat;

use super::kind::ValueKind;
use super::schema::{ExtraParams, ParamSpec, TagSpec};

/// The jQuery-easing keyword set accepted by the `effect` parameter of
/// `anim` and of `chara_config`/`chara_move` (verified against the
/// engine's `:param` doc comments, which enumerate the full list).
/// `three.rs` has its own, genuinely different tween-easing list.
const EASE_EFFECTS: &[&str] = &[
    "jswing", "def", "easeInQuad", "easeOutQuad", "easeInOutQuad", "easeInCubic", "easeOutCubic",
    "easeInOutCubic", "easeInQuart", "easeOutQuart", "easeInOutQuart", "easeInQuint", "easeOutQuint",
    "easeInOutQuint", "easeInSine", "easeOutSine", "easeInOutSine", "easeInExpo", "easeOutExpo",
    "easeInOutExpo", "easeInCirc", "easeOutCirc", "easeInOutCirc", "easeInElastic", "easeOutElastic",
    "easeInOutElastic", "easeInBack", "easeOutBack", "easeInOutBack", "easeInBounce", "easeOutBounce",
    "easeInOutBounce",
];

const fn req(name: &'static str, kind: ValueKind) -> ParamSpec {
    ParamSpec { name, required: true, kind, default: None }
}

const fn opt(name: &'static str, kind: ValueKind) -> ParamSpec {
    ParamSpec { name, required: false, kind, default: None }
}

const fn optd(name: &'static str, kind: ValueKind, default: &'static str) -> ParamSpec {
    ParamSpec { name, required: false, kind, default: Some(default) }
}

const fn tag(
    name: &'static str,
    params: &'static [ParamSpec],
    extra: ExtraParams,
    doc: &'static str,
) -> TagSpec {
    TagSpec { name, params, extra, doc }
}

/// Every builtin tag, grouped by category module. The registry flattens
/// the groups; names are globally unique (enforced at registry build).
pub const BUILTIN_TAG_GROUPS: &[&[TagSpec]] = &[
    anim::TAGS,
    audio::TAGS,
    camera::TAGS,
    chara::TAGS,
    flow::TAGS,
    layer::TAGS,
    message::TAGS,
    system::TAGS,
    three::TAGS,
    vchat::TAGS,
];

/// All builtin tag specs, across every group.
pub fn builtin_tags() -> impl Iterator<Item = &'static TagSpec> {
    BUILTIN_TAG_GROUPS.iter().flat_map(|group| group.iter())
}
