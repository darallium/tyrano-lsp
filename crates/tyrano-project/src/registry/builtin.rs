//! The seed table of builtin TyranoScript tags.
//!
//! Mechanism over coverage: one [`TagSpec`] line per tag, trivially
//! extensible. Parameter lists cover the load-bearing parameters
//! (references, assets, control flow); styling-heavy tags say
//! [`ExtraParams::Allow`] instead of cataloguing their whole tail.

use super::kind::ValueKind;
use super::schema::{ExtraParams, ParamSpec, TagSpec};
use crate::project::AssetKind;

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

use ExtraParams::{Allow, Deny};
use ValueKind::{Asset, Boolean, Expression, Label, Number, Scenario, Text};

/// The builtin tags this analyzer knows out of the box.
pub const BUILTIN_TAGS: &[TagSpec] = &[
    // --- navigation -----------------------------------------------------
    tag(
        "jump",
        &[opt("storage", Scenario), opt("target", Label)],
        Deny,
        "jump to a label, optionally in another scenario file",
    ),
    tag(
        "call",
        &[opt("storage", Scenario), opt("target", Label)],
        Deny,
        "call a subroutine label; [return] comes back",
    ),
    tag(
        "link",
        &[opt("storage", Scenario), opt("target", Label)],
        Allow,
        "begin a text hyperlink to a label",
    ),
    tag("endlink", &[], Deny, "end a [link] region"),
    tag(
        "button",
        &[req("graphic", Asset(AssetKind::Image)), opt("storage", Scenario), opt("target", Label), opt("x", Number), opt("y", Number)],
        Allow,
        "place a graphical jump button",
    ),
    tag(
        "glink",
        &[opt("storage", Scenario), opt("target", Label), opt("text", Text), opt("x", Number), opt("y", Number)],
        Allow,
        "place a styled text link button",
    ),
    tag(
        "clickable",
        &[opt("storage", Scenario), opt("target", Label), opt("width", Number), opt("height", Number), opt("x", Number), opt("y", Number)],
        Allow,
        "place an invisible clickable jump area",
    ),
    tag("s", &[], Deny, "stop scenario progression and wait for input"),
    tag("return", &[], Deny, "return from a [call]"),
    // --- images ----------------------------------------------------------
    tag(
        "image",
        &[req("storage", Asset(AssetKind::Image)), opt("layer", Text), opt("x", Number), opt("y", Number), optd("visible", Boolean, "true"), opt("time", Number)],
        Allow,
        "show an image on a layer",
    ),
    tag(
        "freeimage",
        &[req("layer", Text), opt("time", Number)],
        Allow,
        "clear every image from a layer",
    ),
    tag(
        "bg",
        &[req("storage", Asset(AssetKind::BgImage)), opt("time", Number)],
        Allow,
        "switch the background image",
    ),
    tag(
        "layopt",
        &[req("layer", Text), opt("visible", Boolean), opt("opacity", Number)],
        Allow,
        "change layer options",
    ),
    // --- audio -----------------------------------------------------------
    tag(
        "playbgm",
        &[req("storage", Asset(AssetKind::Bgm)), optd("loop", Boolean, "true"), opt("volume", Number)],
        Allow,
        "play background music",
    ),
    tag("stopbgm", &[opt("time", Number)], Allow, "stop the background music"),
    tag(
        "playse",
        &[req("storage", Asset(AssetKind::Sound)), optd("loop", Boolean, "false"), opt("volume", Number)],
        Allow,
        "play a sound effect",
    ),
    tag("stopse", &[opt("time", Number)], Allow, "stop sound effects"),
    // --- characters -------------------------------------------------------
    tag(
        "chara_new",
        &[req("name", Text), req("storage", Asset(AssetKind::FgImage)), opt("jname", Text)],
        Allow,
        "define a character and their default sprite",
    ),
    tag(
        "chara_show",
        &[req("name", Text), opt("storage", Asset(AssetKind::FgImage)), opt("face", Text), opt("time", Number), opt("wait", Boolean)],
        Allow,
        "show a character sprite",
    ),
    tag(
        "chara_mod",
        &[req("name", Text), opt("storage", Asset(AssetKind::FgImage)), opt("face", Text), opt("time", Number)],
        Allow,
        "change a shown character's sprite",
    ),
    tag(
        "chara_hide",
        &[req("name", Text), opt("time", Number), opt("wait", Boolean)],
        Allow,
        "hide a character sprite",
    ),
    tag("chara_delete", &[req("name", Text)], Allow, "remove a character definition"),
    tag(
        "chara_face",
        &[req("name", Text), req("face", Text), req("storage", Asset(AssetKind::FgImage))],
        Allow,
        "register a face variant for a character",
    ),
    // --- macros ------------------------------------------------------------
    tag("macro", &[req("name", Text)], Deny, "begin a macro definition"),
    tag("endmacro", &[], Deny, "end a macro definition"),
    // --- control flow -------------------------------------------------------
    tag("if", &[req("exp", Expression)], Deny, "begin a conditional block"),
    tag("elsif", &[req("exp", Expression)], Deny, "else-if branch of an [if] block"),
    tag("else", &[], Deny, "else branch of an [if] block"),
    tag("endif", &[], Deny, "end an [if] block"),
    tag("iscript", &[], Deny, "begin an embedded JavaScript block"),
    tag("endscript", &[], Deny, "end an embedded JavaScript block"),
    tag("eval", &[req("exp", Expression)], Deny, "evaluate a JavaScript expression"),
    // --- text & timing --------------------------------------------------------
    tag("wait", &[req("time", Number)], Deny, "pause the scenario for a duration"),
    tag("l", &[], Deny, "wait for a click, then continue inline"),
    tag("p", &[], Deny, "wait for a click, then clear the page"),
    tag("r", &[], Deny, "line break"),
    tag("cm", &[], Deny, "clear the current message layer"),
    tag("ct", &[], Deny, "clear every message layer"),
    tag("er", &[], Deny, "erase the current message layer's text"),
];
