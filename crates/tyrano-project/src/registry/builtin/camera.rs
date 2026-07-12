//! Camera tags: virtual-camera movement and screen masks.

use super::{ExtraParams::{Allow, Deny}, TagSpec, opt, optd, tag};
use super::ValueKind::{Asset, Boolean, Color, Enum, Number, Text};
use crate::project::AssetKind;

pub(super) const TAGS: &[TagSpec] = &[
    tag(
        "camera",
        &[
            optd("time", Number, "1000"),
            opt("from_x", Number),
            opt("from_y", Number),
            opt("from_zoom", Number),
            opt("from_rotate", Number),
            opt("x", Number),
            opt("y", Number),
            opt("zoom", Number),
            opt("rotate", Number),
            optd("layer", Text, "layer_camera"),
            optd("wait", Boolean, "true"),
            optd("ease_type", Enum(&["ease", "linear", "ease-in", "ease-out", "ease-in-out"]), "ease"),
        ],
        Allow,
        "move, zoom, or rotate the virtual camera",
    ),
    tag(
        "reset_camera",
        &[
            optd("time", Number, "1000"),
            optd("wait", Boolean, "true"),
            optd("ease_type", Enum(&["ease", "linear", "ease-in", "ease-out", "ease-in-out"]), "ease"),
            optd("layer", Text, "layer_camera"),
        ],
        Allow,
        "reset the virtual camera to its initial position",
    ),
    tag("wait_camera", &[], Deny, "wait for the current camera effect to finish"),
    tag(
        "mask",
        &[
            optd("time", Number, "1000"),
            optd(
                "effect",
                Enum(&[
                    "fadeIn", "fadeInDownBig", "fadeInLeftBig", "fadeInRightBig", "fadeInUpBig", "flipInX", "flipInY", "lightSpeedIn", "rotateIn",
                    "rotateInDownLeft", "rotateInDownRight", "rotateInUpLeft", "rotateInUpRight", "zoomIn", "zoomInDown", "zoomInLeft", "zoomInRight",
                    "zoomInUp", "slideInDown", "slideInLeft", "slideInRight", "slideInUp", "bounceIn", "bounceInDown", "bounceInLeft", "bounceInRight",
                    "bounceInUp", "rollIn",
                ]),
                "fadeIn",
            ),
            optd("color", Color, "0x000000"),
            opt("graphic", Asset(AssetKind::Image)),
            opt("folder", Text),
        ],
        Allow,
        "darken the screen with a mask, optionally showing an image",
    ),
    tag(
        "mask_off",
        &[
            optd("time", Number, "1000"),
            optd(
                "effect",
                Enum(&[
                    "fadeOut", "fadeOutDownBig", "fadeOutLeftBig", "fadeOutRightBig", "fadeOutUpBig", "flipOutX", "flipOutY", "lightSpeedOut",
                    "rotateOut", "rotateOutDownLeft", "rotateOutDownRight", "rotateOutUpLeft", "rotateOutUpRight", "zoomOut", "zoomOutDown",
                    "zoomOutLeft", "zoomOutRight", "zoomOutUp", "slideOutDown", "slideOutLeft", "slideOutRight", "slideOutUp", "bounceOut",
                    "bounceOutDown", "bounceOutLeft", "bounceOutRight", "bounceOutUp",
                ]),
                "fadeOut",
            ),
        ],
        Allow,
        "remove the screen mask shown by [mask]",
    ),
];
