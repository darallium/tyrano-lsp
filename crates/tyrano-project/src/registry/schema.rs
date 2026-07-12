//! Parameter and tag schemas, plus the registry lookup.

use std::collections::HashMap;
use std::sync::LazyLock;

use super::builtin::BUILTIN_TAG_GROUPS;
use super::kind::ValueKind;

/// One declared parameter of a builtin tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamSpec {
    pub name: &'static str,
    /// Whether omitting the parameter is a diagnostic.
    pub required: bool,
    pub kind: ValueKind,
    /// The engine-side default, when one is documented.
    pub default: Option<&'static str>,
}

/// Whether a tag accepts parameters beyond the declared ones.
///
/// `Allow` is for styling-heavy tags whose long parameter tails are not
/// worth cataloguing; `Deny` turns unknown parameters into diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtraParams {
    Deny,
    Allow,
}

/// The schema of one builtin tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagSpec {
    pub name: &'static str,
    pub params: &'static [ParamSpec],
    pub extra: ExtraParams,
    /// One-line description (hover documentation).
    pub doc: &'static str,
}

impl TagSpec {
    /// The declared parameter named `name`, if any.
    pub fn param(&self, name: &str) -> Option<&'static ParamSpec> {
        self.params.iter().find(|p| p.name == name)
    }
}

/// Parameter names accepted on *every* tag (the engine's universal
/// conditional execution), never reported as unknown.
pub const GLOBAL_PARAMS: &[&str] = &["cond"];

/// Name → [`TagSpec`] lookup over a fixed spec table.
#[derive(Debug)]
pub struct TagRegistry {
    by_name: HashMap<&'static str, &'static TagSpec>,
}

impl TagRegistry {
    /// Builds a registry over the flattened `groups`. Only the spec tables
    /// themselves must be `'static` (the registry stores references into
    /// them); the group list may be built on the fly. Panics on duplicate
    /// tag names — the tables are static data, so that is a programming
    /// error.
    pub fn new(groups: &[&'static [TagSpec]]) -> TagRegistry {
        let mut by_name = HashMap::with_capacity(groups.iter().map(|g| g.len()).sum());
        for spec in groups.iter().flat_map(|group| group.iter()) {
            let prev = by_name.insert(spec.name, spec);
            assert!(prev.is_none(), "duplicate tag spec `{}`", spec.name);
        }
        TagRegistry { by_name }
    }

    /// The spec for tag `name`, if it is a known builtin.
    pub fn get(&self, name: &str) -> Option<&'static TagSpec> {
        self.by_name.get(name).copied()
    }

    /// All registered tag names, unordered.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_name.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// The registry over [`BUILTIN_TAG_GROUPS`].
pub fn builtin_registry() -> &'static TagRegistry {
    static REGISTRY: LazyLock<TagRegistry> =
        LazyLock::new(|| TagRegistry::new(BUILTIN_TAG_GROUPS));
    &REGISTRY
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::AssetKind;

    #[test]
    fn lookup_finds_specs_and_params() {
        let reg = builtin_registry();
        let jump = reg.get("jump").expect("jump is builtin");
        assert_eq!(jump.extra, ExtraParams::Deny);
        let target = jump.param("target").expect("jump has target=");
        assert_eq!(target.kind, ValueKind::Label);
        assert!(!target.required);
        assert_eq!(jump.param("nope"), None);
        assert_eq!(reg.get("no_such_tag"), None);
    }

    #[test]
    fn asset_params_carry_their_namespace() {
        let reg = builtin_registry();
        assert_eq!(
            reg.get("playbgm").unwrap().param("storage").unwrap().kind,
            ValueKind::Asset(AssetKind::Bgm)
        );
        assert_eq!(
            reg.get("bg").unwrap().param("storage").unwrap().kind,
            ValueKind::Asset(AssetKind::BgImage)
        );
    }

    #[test]
    fn registry_builds_from_a_borrowed_group_list() {
        // Only the spec tables are 'static; the group list is a local.
        const SPECS: &[TagSpec] =
            &[TagSpec { name: "local", params: &[], extra: ExtraParams::Deny, doc: "a tag" }];
        let groups = vec![SPECS];
        let reg = TagRegistry::new(&groups);
        assert!(reg.get("local").is_some());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn seed_set_is_present() {
        let reg = builtin_registry();
        for name in ["jump", "call", "macro", "endmacro", "if", "endif", "l", "p", "s", "eval"] {
            assert!(reg.get(name).is_some(), "`{name}` missing from builtin table");
        }
        for spec in crate::registry::builtin_tags() {
            assert!(!spec.doc.is_empty(), "`{}` has no doc line", spec.name);
        }
    }

    /// Pins the complete tag inventory: exactly the tags defined by the
    /// reference engine (`tyrano/plugins/kag/kag.tag*.js`, TyranoScript
    /// master), no more, no fewer.
    #[test]
    fn full_inventory_matches_the_reference_engine() {
        // Generated from the TyranoScript master sources (kag.tag*.js).
        const ALL: &[&str] = &[
            "3d_add_group", "3d_anim", "3d_anim_stop", "3d_bg360", "3d_bg360_video", "3d_box_new",
            "3d_camera", "3d_canvas_hide", "3d_canvas_show", "3d_clone", "3d_close",
            "3d_cylinder_new", "3d_debug", "3d_debug_bk", "3d_debug_camera", "3d_delete",
            "3d_delete_all", "3d_event", "3d_event_delete", "3d_event_start", "3d_event_stop",
            "3d_fps_control", "3d_gyro", "3d_gyro_stop", "3d_helper", "3d_hide", "3d_hide_all",
            "3d_html_new", "3d_image_new", "3d_init", "3d_model_mod", "3d_model_new", "3d_motion",
            "3d_new_group", "3d_point_light_new", "3d_scene", "3d_show", "3d_sound",
            "3d_sound_play", "3d_sound_stop", "3d_sphere_new", "3d_spot_light_new", "3d_sprite_mod",
            "3d_sprite_new", "3d_text_mod", "3d_text_mod_old", "3d_text_new", "3d_text_new_old",
            "3d_video_play", "_s", "anim", "apply_local_patch", "autoconfig", "autoload",
            "autosave", "autostart", "autostop", "awakegame", "backlay", "bg", "bg2", "bgmopt",
            "bgmovie", "body", "breakgame", "button", "call", "camera", "cancelskip", "changevol",
            "chara_config", "chara_delete", "chara_face", "chara_hide", "chara_hide_all",
            "chara_layer", "chara_layer_mod", "chara_mod", "chara_move", "chara_new", "chara_part",
            "chara_part_reset", "chara_ptext", "chara_show", "check_web_patch", "checkpoint",
            "clear_checkpoint", "clearfix", "clearstack", "clearsysvar", "clearvar", "clickable",
            "close", "closeconfirm_off", "closeconfirm_on", "cm", "commit", "config_record_label",
            "configdelay", "ct", "current", "cursor", "deffont", "delay", "dialog", "dialog_config",
            "dialog_config_filter", "dialog_config_ng", "dialog_config_ok", "edit", "else", "elsif",
            "emb", "endhtml", "endif", "endignore", "endkeyframe", "endlink", "endmacro", "endmark",
            "endnolog", "endnowait", "endscript", "er", "erasemacro", "eval", "fadeinbgm",
            "fadeinse", "fadeoutbgm", "fadeoutse", "filter", "font", "fps_control_start",
            "fps_control_stop", "frame", "free", "free_filter", "free_layermode", "freeimage",
            "freelayer", "fuki_chara", "fuki_start", "fuki_stop", "glink", "glink_config", "glyph",
            "glyph_auto", "glyph_skip", "graph", "hidemenubutton", "hidemessage", "html", "if",
            "ignore", "image", "iscript", "jump", "kanim", "keyframe", "l", "label", "lang_set",
            "layermode", "layermode_movie", "layopt", "link", "loadcss", "loading_log", "loadjs",
            "locate", "macro", "mark", "mask", "mask_off", "message_config", "mode_effect", "movie",
            "mtext", "nolog", "nowait", "obj_model_mod", "obj_model_new", "p", "pausebgm",
            "pausese", "playbgm", "playse", "plugin", "popopo", "position", "position_filter",
            "preload", "ptext", "pushlog", "quake", "quake2", "r", "reset_camera", "resetdelay",
            "resetfont", "resumebgm", "resumese", "return", "rollback", "ruby", "s", "save_img",
            "savesnap", "screen_full", "seopt", "set_resizecall", "showload", "showlog", "showmenu",
            "showmenubutton", "showsave", "skipstart", "skipstop", "sleepgame", "speak_off",
            "speak_on", "start_keyconfig", "stop_bgmovie", "stop_kanim", "stop_keyconfig",
            "stop_xanim", "stopanim", "stopbgm", "stopse", "sysview", "text", "title", "trace",
            "trans", "unload", "vchat_chara", "vchat_config", "vchat_in", "vibrate", "vibrate_stop",
            "voconfig", "vostart", "vostop", "wa", "wait", "wait_bgmovie", "wait_camera",
            "wait_cancel", "wait_preload", "wb", "wbgm", "web", "wse", "wt", "xanim", "xchgbgm",
        ];
        let reg = builtin_registry();
        for name in ALL {
            assert!(reg.get(name).is_some(), "`{name}` missing from builtin table");
        }
        assert_eq!(
            reg.len(),
            ALL.len(),
            "registry has tags outside the reference inventory: {:?}",
            reg.names().filter(|n| !ALL.contains(n)).collect::<Vec<_>>()
        );
    }
}
