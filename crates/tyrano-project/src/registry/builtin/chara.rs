//! Character tags: the `chara_*` family for defining, showing, moving,
//! and layering character sprites.

use super::{EASE_EFFECTS, ExtraParams::Allow, TagSpec, opt, optd, req, tag};
use super::ValueKind::{Asset, Boolean, Enum, Number, Text};
use crate::project::AssetKind;

pub(super) const TAGS: &[TagSpec] = &[
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
    tag(
        "chara_config",
        &[
            opt("pos_mode", Boolean),
            opt("effect", Enum(EASE_EFFECTS)),
            opt("ptext", Text),
            opt("time", Number),
            opt("memory", Boolean),
            opt("anim", Boolean),
            opt("pos_change_time", Number),
            opt("talk_focus", Enum(&["brightness", "blur", "none"])),
            opt("brightness_value", Number),
            opt("blur_value", Number),
            opt("talk_anim", Enum(&["up", "down", "zoom", "none"])),
            opt("talk_anim_time", Number),
            opt("talk_anim_value", Number),
            opt("talk_anim_zoom_rate", Number),
            opt("plus_lighter", Text),
        ],
        Allow,
        "configure default behavior for the chara_* tag family",
    ),
    tag(
        "chara_hide_all",
        &[optd("page", Enum(&["fore", "back"]), "fore"), optd("layer", Text, "0"), optd("wait", Boolean, "true"), optd("time", Number, "1000")],
        Allow,
        "hide every shown character sprite",
    ),
    tag(
        "chara_move",
        &[
            req("name", Text),
            optd("time", Number, "600"),
            optd("anim", Boolean, "false"),
            opt("left", Text),
            opt("top", Text),
            opt("width", Number),
            opt("height", Number),
            opt("effect", Enum(EASE_EFFECTS)),
            optd("wait", Boolean, "true"),
        ],
        Allow,
        "move or resize a shown character sprite",
    ),
    tag(
        "chara_layer",
        &[req("name", Text), req("part", Text), req("id", Text), opt("storage", Asset(AssetKind::FgImage)), opt("zindex", Number)],
        Allow,
        "define a swappable differential part for a character sprite",
    ),
    tag(
        "chara_layer_mod",
        &[req("name", Text), req("part", Text), opt("zindex", Number)],
        Allow,
        "change a character differential part's stacking order",
    ),
    tag(
        "chara_part",
        &[req("name", Text), optd("allow_storage", Boolean, "false"), opt("time", Number), optd("wait", Boolean, "true"), optd("force", Boolean, "false")],
        Allow,
        "switch a character's differential parts (e.g. face, eyes)",
    ),
    tag(
        "chara_part_reset",
        &[req("name", Text), opt("part", Text)],
        Allow,
        "reset a character's differential parts to their defaults",
    ),
    tag(
        "chara_ptext",
        &[opt("name", Text), opt("face", Text)],
        Allow,
        "display a character's name and optionally change their face",
    ),
];
