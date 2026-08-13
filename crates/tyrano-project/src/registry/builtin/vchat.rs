//! Virtual-chat tags: the `vchat_*` family.

use super::ValueKind::{Color, Text};
use super::{ExtraParams::Allow, TagSpec, opt, req, tag};

pub(super) const TAGS: &[TagSpec] = &[
    tag(
        "vchat_chara",
        &[req("name", Text), opt("color", Color)],
        Allow,
        "register a virtual-chat character's balloon color",
    ),
    tag(
        "vchat_config",
        &[opt("chara_name_color", Color)],
        Allow,
        "configure virtual-chat display options",
    ),
    tag(
        "vchat_in",
        &[],
        Allow,
        "place a virtual-chat balloon (internal use)",
    ),
];
