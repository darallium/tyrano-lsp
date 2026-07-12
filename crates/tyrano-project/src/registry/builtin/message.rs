//! Message and text tags: click waits, page control, message-layer
//! clearing, fonts, glyphs, speech bubbles, and backlog control.

use super::{ExtraParams::{Allow, Deny}, TagSpec, opt, optd, req, tag};
use super::ValueKind::{Asset, Boolean, Color, Enum, Label, Number, Text};
use crate::project::AssetKind;

pub(super) const TAGS: &[TagSpec] = &[
    tag("l", &[], Deny, "wait for a click, then continue inline"),
    tag("p", &[], Deny, "wait for a click, then clear the page"),
    tag("r", &[], Deny, "line break"),
    tag("cm", &[optd("next", Boolean, "true")], Deny, "clear the current message layer"),
    tag("ct", &[], Deny, "clear every message layer"),
    tag("er", &[], Deny, "erase the current message layer's text"),

    // Text display speed and instant-display toggles.
    tag("nowait", &[], Deny, "start instant text-display mode"),
    tag("endnowait", &[], Deny, "stop instant text-display mode"),
    tag("delay", &[opt("speed", Number)], Deny, "set the character display speed"),
    tag("resetdelay", &[opt("speed", Number)], Deny, "reset the character display speed to default"),
    tag("configdelay", &[opt("speed", Number)], Deny, "set the default character display speed"),

    // Text style and markers.
    tag("font", &[], Allow, "change the text style"),
    tag("deffont", &[], Allow, "set the default text style"),
    tag("resetfont", &[optd("next", Boolean, "true")], Deny, "reset the text style"),
    tag(
        "mark",
        &[optd("color", Color, "0xFFFF00"), opt("font_color", Color), opt("size", Number)],
        Allow,
        "start a text marker/highlight",
    ),
    tag("endmark", &[], Deny, "end a text marker"),

    // Drawing position and message-layer selection.
    tag("locate", &[opt("x", Number), opt("y", Number)], Deny, "set the text drawing position"),
    tag(
        "current",
        &[opt("layer", Text), optd("page", Enum(&["fore", "back"]), "fore")],
        Deny,
        "select the message layer to operate on",
    ),

    // Message window configuration.
    tag(
        "position",
        &[
            optd("layer", Text, "message0"), optd("page", Enum(&["fore", "back"]), "fore"),
            opt("left", Number), opt("top", Number), opt("width", Number), opt("height", Number),
            opt("color", Color), opt("opacity", Number), opt("vertical", Boolean), opt("frame", Text),
            opt("radius", Number), opt("border_color", Color), opt("border_size", Number),
            opt("marginl", Number), opt("margint", Number), opt("marginr", Number), opt("marginb", Number),
            opt("margin", Text), opt("gradient", Text), opt("visible", Boolean), optd("next", Boolean, "true"),
        ],
        Allow,
        "change message-window attributes",
    ),
    tag("message_config", &[], Allow, "configure message-window behaviour"),
    tag("hidemessage", &[], Deny, "temporarily hide the message layers"),

    // Click-wait / mode glyphs.
    tag(
        "glyph",
        &[
            optd("line", Text, "nextpage.gif"), optd("layer", Text, "message0"), optd("fix", Boolean, "false"),
            optd("left", Number, "0"), optd("top", Number, "0"), opt("name", Text),
            optd("folder", Text, "tyrano/images/system"), opt("width", Number), opt("height", Number),
            opt("anim", Text), opt("time", Number), opt("figure", Text), optd("color", Color, "0xFFFFFF"),
            opt("html", Text), optd("marginl", Number, "3"), optd("marginb", Number, "0"), opt("keyframe", Text),
            opt("easing", Text), opt("count", Number), opt("delay", Number), opt("derection", Text), opt("mode", Text),
            opt("koma_anim", Text), opt("koma_count", Number), opt("koma_width", Number),
            optd("koma_anim_time", Number, "1000"), opt("target", Label),
        ],
        Allow,
        "configure the click-wait glyph",
    ),
    tag("glyph_auto", &[], Allow, "configure the auto-mode glyph"),
    tag("glyph_skip", &[], Allow, "configure the skip-mode glyph"),

    // Inline image and decorative text.
    tag("graph", &[req("storage", Asset(AssetKind::Image))], Allow, "display an inline image"),
    tag(
        "mtext",
        &[
            req("x", Number), req("y", Number),
            optd("layer", Text, "0"), optd("page", Enum(&["fore", "back"]), "fore"), optd("vertical", Boolean, "false"),
            opt("text", Text), opt("size", Number), opt("face", Text), opt("color", Color), opt("italic", Text),
            opt("bold", Text), opt("shadow", Color), opt("edge", Text), opt("name", Text), optd("zindex", Number, "9999"),
            opt("width", Number), optd("align", Enum(&["left", "center", "right"]), "left"),
            optd("fadeout", Boolean, "true"), optd("time", Number, "2000"), optd("in_effect", Text, "fadeIn"),
            optd("in_delay", Number, "50"), optd("in_delay_scale", Number, "1.5"), optd("in_sync", Boolean, "false"),
            optd("in_shuffle", Boolean, "false"), optd("in_reverse", Boolean, "false"), optd("wait", Boolean, "true"),
            optd("out_effect", Text, "fadeOut"), optd("out_delay", Number, "50"), opt("out_scale_delay", Number),
            optd("out_sync", Boolean, "false"), optd("out_shuffle", Boolean, "false"), optd("out_reverse", Boolean, "false"),
        ],
        Allow,
        "display animated decorative text",
    ),
    tag("ruby", &[req("text", Text)], Deny, "attach ruby text to the following characters"),
    tag("text", &[opt("val", Text), optd("backlog", Text, "add")], Deny, "append text to the message layer"),

    // Mouse cursor image.
    tag(
        "cursor",
        &[
            opt("storage", Asset(AssetKind::Image)), optd("x", Number, "0"), optd("y", Number, "0"),
            optd("type", Text, "default"), opt("click_effect", Boolean), opt("mousedown_effect", Boolean),
            opt("touch_effect", Boolean), optd("next", Boolean, "true"),
        ],
        Deny,
        "set the mouse-cursor image",
    ),

    // Speech bubbles.
    tag(
        "fuki_chara",
        &[
            req("name", Text), optd("sippo", Enum(&["top", "bottom", "left", "right"]), "top"),
            optd("sippo_left", Number, "40"), optd("sippo_top", Number, "40"), optd("sippo_width", Number, "12"),
            optd("sippo_height", Number, "20"), optd("enable", Boolean, "true"), optd("max_width", Number, "300"),
            opt("fix_width", Number), opt("font_color", Color), opt("font_size", Number), opt("color", Color),
            opt("opacity", Number), opt("border_size", Number), opt("border_color", Color), opt("radius", Number),
        ],
        Allow,
        "register a character for speech bubbles",
    ),
    tag(
        "fuki_start",
        &[optd("layer", Text, "message0"), optd("page", Enum(&["fore", "back"]), "fore")],
        Allow,
        "turn a message layer into a speech bubble",
    ),
    tag("fuki_stop", &[], Allow, "disable speech-bubble mode on the message layer"),

    // Backlog control.
    tag("nolog", &[], Deny, "pause backlog recording"),
    tag("endnolog", &[], Deny, "resume backlog recording"),
    tag("pushlog", &[req("text", Text), optd("join", Boolean, "false")], Deny, "append text to the backlog"),
    tag(
        "loading_log",
        &[opt("mintime", Number), opt("all", Text), opt("load", Text), opt("save", Text), opt("dottime", Number)],
        Deny,
        "configure the loading log",
    ),
];
