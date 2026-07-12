//! Animation tags: tweens, keyframe animations, screen shakes, and
//! their stop/wait counterparts.

use super::{EASE_EFFECTS, ExtraParams::{Allow, Deny}, TagSpec, opt, optd, req, tag};
use super::ValueKind::{Asset, Boolean, Color, Enum, Number, Text};
use crate::project::AssetKind;

pub(super) const TAGS: &[TagSpec] = &[
    tag(
        "anim",
        &[
            opt("name", Text),
            opt("layer", Text),
            opt("left", Text),
            opt("top", Text),
            opt("width", Number),
            opt("height", Number),
            opt("opacity", Number),
            opt("color", Color),
            optd("time", Number, "2000"),
            opt("effect", Enum(EASE_EFFECTS)),
        ],
        Allow,
        "tween an image, button, or layer's position/size/opacity/color",
    ),
    tag("keyframe", &[req("name", Text)], Allow, "begin a keyframe animation definition"),
    tag("endkeyframe", &[], Allow, "end a keyframe animation definition"),
    tag(
        "kanim",
        &[req("keyframe", Text), opt("name", Text), opt("layer", Text)],
        Allow,
        "play a defined keyframe animation on a target",
    ),
    tag(
        "xanim",
        &[
            opt("name", Text),
            opt("layer", Text),
            opt("keyframe", Text),
            optd("easing", Text, "ease"),
            optd("count", Text, "1"),
            optd("delay", Number, "0"),
            optd("direction", Text, "normal"),
            optd("mode", Enum(&["forwards", "none"]), "forwards"),
            optd("reset", Boolean, "true"),
            opt("time", Number),
            opt("svg", Asset(AssetKind::Image)),
            optd("svg_x", Boolean, "true"),
            optd("svg_y", Boolean, "true"),
            optd("svg_rotate", Boolean, "false"),
            optd("next", Boolean, "true"),
            optd("wait", Boolean, "false"),
        ],
        Allow,
        "play a general-purpose animation (tween or keyframe-based)",
    ),
    tag("stopanim", &[req("name", Text)], Deny, "forcibly stop a running [anim] animation"),
    tag("stop_kanim", &[opt("name", Text), opt("layer", Text)], Deny, "stop a running keyframe animation"),
    tag(
        "stop_xanim",
        &[opt("name", Text), opt("layer", Text), optd("complete", Boolean, "false")],
        Deny,
        "stop a running [xanim] animation",
    ),
    tag("wa", &[], Deny, "wait for all running animations to finish"),
    tag(
        "quake",
        &[req("time", Number), optd("count", Number, "5"), opt("timemode", Text), optd("hmax", Number, "0"), optd("vmax", Number, "10"), optd("wait", Boolean, "true")],
        Deny,
        "shake the screen for a duration",
    ),
    tag(
        "quake2",
        &[optd("time", Number, "1000"), optd("hmax", Number, "0"), optd("vmax", Number, "200"), optd("wait", Boolean, "true"), optd("copybase", Boolean, "true"), optd("skippable", Boolean, "true")],
        Deny,
        "shake the screen (v2, smoother continuous shake)",
    ),
    tag(
        "vibrate",
        &[optd("time", Text, "500"), optd("power", Number, "100"), opt("count", Number)],
        Deny,
        "vibrate the mobile device or gamepad",
    ),
    tag("vibrate_stop", &[], Deny, "stop device/gamepad vibration"),
];
