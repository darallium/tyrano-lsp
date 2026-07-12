//! System tags: save/load, game suspend/resume, dialogs, variable
//! clearing, plugins, HTML/edit widgets, key configuration, patches, and
//! window control.

use super::{ExtraParams::{Allow, Deny}, TagSpec, opt, optd, req, tag};
use super::ValueKind::{Boolean, Color, Enum, Expression, Label, Number, Scenario, Text, VariableName};

pub(super) const TAGS: &[TagSpec] = &[
    // --- save / load / game state ---------------------------------------
    tag("autosave", &[opt("title", Text)], Deny, "run an autosave"),
    tag("autoload", &[opt("title", Text)], Deny, "run an autoload"),
    tag(
        "save_img",
        &[opt("storage", Text), opt("folder", Text)],
        Deny,
        "set the image used for save-data thumbnails",
    ),
    tag("savesnap", &[req("title", Text)], Deny, "create a save snapshot"),

    // --- window / app control -------------------------------------------
    tag("close", &[optd("ask", Boolean, "true")], Deny, "close the game window"),
    tag("closeconfirm_on", &[], Deny, "enable the exit confirmation"),
    tag("closeconfirm_off", &[], Deny, "disable the exit confirmation"),
    tag("breakgame", &[], Deny, "delete the game suspend data"),
    tag(
        "sleepgame",
        &[opt("storage", Scenario), opt("target", Label), optd("next", Boolean, "true")],
        Deny,
        "suspend the game until [awakegame]",
    ),
    tag(
        "awakegame",
        &[optd("variable_over", Boolean, "true"), optd("sound_opt_over", Boolean, "true"), optd("bgm_over", Boolean, "true")],
        Deny,
        "resume a game suspended by [sleepgame]",
    ),
    tag("screen_full", &[], Deny, "toggle fullscreen"),
    tag("commit", &[], Deny, "commit form input"),

    // --- variables ------------------------------------------------------
    tag("clearvar", &[opt("exp", Expression)], Deny, "clear a game variable (all when omitted)"),
    tag("clearsysvar", &[], Deny, "clear all system variables"),
    tag("clearfix", &[opt("name", Text)], Deny, "clear the fix layer"),
    tag(
        "clearstack",
        &[opt("stack", Enum(&["call", "if", "macro", "anim"]))],
        Deny,
        "clear execution stacks",
    ),
    tag("trace", &[opt("exp", Expression)], Deny, "print a value to the console"),

    // --- dialogs --------------------------------------------------------
    tag(
        "dialog",
        &[
            optd("name", VariableName, "tf.dialog_value"),
            optd("type", Enum(&["alert", "confirm", "input"]), "alert"),
            opt("text", Text),
            opt("storage", Scenario),
            opt("target", Label),
            opt("storage_cancel", Scenario),
            opt("target_cancel", Label),
            optd("label_ok", Text, "OK"),
            optd("label_cancel", Text, "Cancel"),
        ],
        Allow,
        "show a browser dialog (alert/confirm/input)",
    ),
    tag(
        "dialog_config",
        &[
            opt("okpos", Text), opt("btntype", Text), opt("btnwidth", Number),
            opt("btnmargin", Text), opt("btnpadding", Text), opt("fontsize", Number),
            opt("fontbold", Boolean), opt("fontface", Text), opt("fontcolor", Color),
            opt("btnfontsize", Number), opt("btnfontbold", Boolean), opt("btnfontface", Text),
            opt("btnfontcolor", Color), opt("boxradius", Number), opt("boxcolor", Color),
            opt("boximg", Text), opt("boximgpos", Text), opt("boximgrepeat", Text),
            opt("boximgsize", Text), opt("boxopacity", Number), opt("boxwidth", Number),
            opt("boxheight", Number), opt("boxpadding", Text), opt("bgcolor", Color),
            opt("bgimg", Text), opt("bgimgpos", Text), opt("bgimgrepeat", Text),
            opt("bgimgsize", Text), opt("bgopacity", Number), opt("openeffect", Text),
            opt("opentime", Number), opt("closeeffect", Text), opt("closetime", Number),
            opt("gotitle", Text), opt("ingame", Text),
        ],
        Allow,
        "configure the confirm-dialog design",
    ),
    tag("dialog_config_filter", &[], Allow, "configure the confirm-dialog backdrop filter"),
    tag(
        "dialog_config_ok",
        &[
            opt("text", Text), opt("type", Text), opt("width", Number), opt("margin", Text),
            opt("padding", Text), opt("fontsize", Number), opt("fontbold", Boolean),
            opt("fontface", Text), opt("fontcolor", Color), opt("img", Text),
            opt("imgwidth", Number), opt("enterimg", Text), opt("activeimg", Text),
            opt("clickimg", Text), opt("enterse", Text), opt("leavese", Text),
            opt("clickse", Text), opt("btnimgtype", Text),
        ],
        Allow,
        "configure the confirm-dialog OK button",
    ),
    tag(
        "dialog_config_ng",
        &[
            opt("text", Text), opt("type", Text), opt("width", Number), opt("margin", Text),
            opt("padding", Text), opt("fontsize", Number), opt("fontbold", Boolean),
            opt("fontface", Text), opt("fontcolor", Color), opt("img", Text),
            opt("imgwidth", Number), opt("enterimg", Text), opt("activeimg", Text),
            opt("clickimg", Text), opt("enterse", Text), opt("leavese", Text),
            opt("clickse", Text), opt("btnimgtype", Text),
        ],
        Allow,
        "configure the confirm-dialog cancel button",
    ),

    // --- menu / system screens ------------------------------------------
    tag("showmenubutton", &[optd("keyfocus", Text, "false")], Deny, "show the menu button"),
    tag("hidemenubutton", &[], Deny, "hide the menu button"),
    tag("showmenu", &[], Deny, "show the menu screen"),
    tag("showsave", &[], Deny, "show the save screen"),
    tag("showload", &[], Deny, "show the load screen"),
    tag("showlog", &[], Deny, "show the backlog"),
    tag(
        "sysview",
        &[req("type", Enum(&["save", "load", "backlog", "menu"])), req("storage", Text)],
        Deny,
        "override a system screen's HTML",
    ),

    // --- HTML / input / web ---------------------------------------------
    tag(
        "html",
        &[opt("layer", Text), optd("top", Number, "0"), optd("left", Number, "0")],
        Allow,
        "add an HTML region to a layer",
    ),
    tag("endhtml", &[], Deny, "end an [html] region"),
    tag(
        "edit",
        &[
            req("name", VariableName),
            opt("length", Number),
            opt("initial", Text),
            opt("placeholder", Text),
            optd("color", Text, "black"),
            optd("left", Number, "0"),
            optd("top", Number, "0"),
            optd("size", Number, "20"),
            opt("face", Text),
            optd("width", Number, "200"),
            optd("autocomplete", Boolean, "false"),
            optd("height", Number, "40"),
            optd("maxchars", Number, "1000"),
        ],
        Allow,
        "place a text input box",
    ),
    tag("web", &[req("url", Text)], Deny, "open a web page or external URL"),

    // --- loading / plugins / patches ------------------------------------
    tag(
        "loadjs",
        &[req("storage", Text), opt("type", Text)],
        Deny,
        "load an external JavaScript file",
    ),
    tag("loadcss", &[req("file", Text)], Deny, "load an external CSS file"),
    tag(
        "plugin",
        &[req("name", Text), optd("storage", Text, "init.ks")],
        Allow,
        "load a plugin and forward parameters to its init",
    ),
    tag(
        "preload",
        &[req("storage", Text), optd("wait", Boolean, "false"), optd("single_use", Boolean, "true"), opt("name", Text)],
        Deny,
        "preload asset files ahead of use",
    ),
    tag(
        "unload",
        &[opt("storage", Text), opt("name", Text), optd("all_sound", Boolean, "false")],
        Deny,
        "discard preloaded audio data",
    ),
    tag("wait_preload", &[], Deny, "wait for [preload] to finish"),
    tag(
        "apply_local_patch",
        &[req("file", Text), optd("reload", Boolean, "false")],
        Deny,
        "apply a local patch file",
    ),
    tag(
        "check_web_patch",
        &[req("url", Text), optd("reload", Boolean, "false")],
        Deny,
        "check for a web update patch",
    ),
    tag(
        "set_resizecall",
        &[req("storage", Scenario), opt("target", Label)],
        Deny,
        "register a jump called on window resize",
    ),
    tag("start_keyconfig", &[], Deny, "enable key-config operation"),
    tag("stop_keyconfig", &[], Deny, "disable key-config operation"),

    // --- misc -----------------------------------------------------------
    tag(
        "body",
        &[opt("bgimage", Text), opt("bgrepeat", Text), opt("bgcolor", Color), optd("bgcover", Boolean, "false"), opt("scWidth", Number), opt("scHeight", Number)],
        Deny,
        "configure the area outside the game screen",
    ),
    tag("title", &[req("name", Text)], Deny, "set the window / browser title"),
    tag("lang_set", &[req("name", Text)], Deny, "switch the localization language"),
];
