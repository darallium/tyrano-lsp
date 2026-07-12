//! Image, layer, and transition tags: backgrounds, layer images, free
//! objects, masks, filters, layer modes, and movie playback.

use super::{ExtraParams::{Allow, Deny}, TagSpec, opt, optd, req, tag};
use super::ValueKind::{Asset, Boolean, Color, Enum, Number, Text};
use crate::project::AssetKind;

pub(super) const TAGS: &[TagSpec] = &[
    tag(
        // `storage` is not in the engine's vital array, but the official
        // reference treats it as required and an [image] without it is
        // always a script bug, so the registry keeps it required.
        "image",
        &[
            req("storage", Asset(AssetKind::Image)), optd("layer", Text, "base"), optd("page", Enum(&["fore", "back"]), "fore"),
            opt("x", Number), opt("y", Number), opt("top", Number), opt("left", Number), opt("width", Number),
            opt("height", Number), opt("pos", Text), optd("visible", Boolean, "true"), opt("time", Number),
            optd("wait", Boolean, "true"), opt("name", Text), opt("folder", Text), optd("depth", Text, "front"),
            opt("reflect", Boolean), optd("zindex", Number, "1"),
        ],
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

    // Page copy.
    tag("backlay", &[opt("layer", Text)], Deny, "copy layer state from the fore page to the back page"),

    // Background switching.
    tag(
        "bg2",
        &[
            req("storage", Asset(AssetKind::BgImage)), opt("name", Text), optd("method", Text, "crossfade"),
            optd("wait", Boolean, "true"), optd("time", Number, "3000"), opt("width", Number), opt("height", Number),
            opt("left", Number), opt("top", Number), optd("cross", Boolean, "false"),
        ],
        Allow,
        "switch a named background image",
    ),

    // Transitions.
    tag(
        "trans",
        &[req("time", Number), req("layer", Text), optd("method", Text, "fadeIn"), optd("children", Boolean, "false")],
        Allow,
        "run a layer transition (accepts a rule image)",
    ),

    // Layer text.
    tag(
        "ptext",
        &[
            req("layer", Text), req("x", Number), req("y", Number),
            optd("page", Enum(&["fore", "back"]), "fore"), optd("vertical", Boolean, "false"), opt("text", Text),
            opt("size", Number), opt("face", Text), opt("color", Color), opt("italic", Text), opt("bold", Text),
            optd("align", Enum(&["left", "center", "right"]), "left"), opt("edge", Text), opt("shadow", Color),
            opt("name", Text), opt("time", Number), opt("width", Number), optd("zindex", Number, "9999"),
            optd("overwrite", Boolean, "false"),
        ],
        Allow,
        "display text on a layer",
    ),

    // Keyframe animation definition.
    tag("frame", &[req("p", Text)], Allow, "define a keyframe-animation step"),

    // Freeing objects and layers.
    tag(
        "free",
        &[
            req("layer", Text), req("name", Text), optd("page", Enum(&["fore", "back"]), "fore"),
            optd("wait", Boolean, "true"), opt("time", Number),
        ],
        Deny,
        "release a named object from a layer",
    ),
    tag(
        "freelayer",
        &[req("layer", Text), optd("page", Enum(&["fore", "back"]), "fore"), opt("time", Number), optd("wait", Boolean, "true")],
        Deny,
        "clear a layer",
    ),

    // Filter effects.
    tag(
        "filter",
        &[
            optd("layer", Text, "all"), optd("page", Enum(&["fore", "back"]), "fore"), opt("name", Text),
            opt("grayscale", Number), opt("sepia", Number), opt("saturate", Number), opt("hue", Number),
            opt("invert", Number), opt("opacity", Number), opt("brightness", Number), opt("contrast", Number),
            opt("blur", Number),
        ],
        Deny,
        "apply a filter effect to a layer",
    ),
    tag(
        "free_filter",
        &[opt("layer", Text), optd("page", Enum(&["fore", "back"]), "fore"), opt("name", Text)],
        Deny,
        "remove a filter effect",
    ),
    tag(
        "position_filter",
        &[
            optd("layer", Text, "message0"), optd("page", Enum(&["fore", "back"]), "fore"), optd("remove", Boolean, "false"),
            opt("grayscale", Number), opt("sepia", Number), opt("saturate", Number), opt("hue", Number),
            opt("invert", Number), opt("opacity", Number), opt("brightness", Number), opt("contrast", Number),
            opt("blur", Number),
        ],
        Deny,
        "apply a filter effect behind the message window",
    ),

    // Layer modes (blend compositing).
    tag(
        "layermode",
        &[
            opt("name", Text), opt("graphic", Asset(AssetKind::Image)), opt("color", Color),
            optd("mode", Text, "multiply"), opt("folder", Text), opt("opacity", Number), optd("time", Number, "500"),
            optd("wait", Boolean, "true"),
        ],
        Allow,
        "create a blend-mode compositing layer",
    ),
    tag(
        "free_layermode",
        &[opt("name", Text), optd("time", Number, "500"), optd("wait", Boolean, "true")],
        Allow,
        "remove a compositing layer",
    ),
    tag(
        "layermode_movie",
        &[
            req("video", Asset(AssetKind::Video)), opt("name", Text), optd("mode", Text, "multiply"), opt("opacity", Number),
            optd("time", Number, "500"), optd("wait", Boolean, "false"), opt("volume", Number), optd("loop", Boolean, "true"),
            optd("mute", Boolean, "false"), opt("speed", Number), optd("fit", Boolean, "true"), opt("width", Number),
            opt("height", Number), opt("top", Number), opt("left", Number), optd("stop", Boolean, "false"),
        ],
        Allow,
        "create a video blend-mode compositing layer",
    ),

    // Mode-change effects.
    tag(
        "mode_effect",
        &[opt("all", Text), opt("skip", Text), opt("auto", Text), opt("holdskip", Text), opt("stop", Text), optd("env", Text, "all")],
        Allow,
        "set the effects used on game-mode changes",
    ),

    // Movie playback.
    tag(
        "movie",
        &[
            req("storage", Asset(AssetKind::Video)), opt("volume", Number), optd("skip", Boolean, "false"),
            optd("mute", Boolean, "false"), optd("bgmode", Boolean, "false"), optd("loop", Boolean, "false"),
        ],
        Allow,
        "play a video",
    ),
    tag(
        "bgmovie",
        &[
            req("storage", Asset(AssetKind::Video)), opt("volume", Number), optd("loop", Boolean, "true"),
            optd("mute", Boolean, "false"), optd("time", Number, "300"), optd("stop", Boolean, "false"),
        ],
        Allow,
        "play a background video",
    ),
    tag("stop_bgmovie", &[optd("time", Number, "300"), optd("wait", Boolean, "true")], Deny, "stop the background video"),
    tag("wait_bgmovie", &[optd("stop", Boolean, "false")], Deny, "wait for the background video to finish"),

    // Transition waits.
    tag("wt", &[], Deny, "wait for a transition to finish"),
    tag("wb", &[], Deny, "wait for the back-page transition to finish"),
];
