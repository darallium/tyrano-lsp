use serde::{Deserialize, Serialize};

/// Whitespace handling for tag parameter values, mirroring TyranoScript's
/// `KeepSpaceInParameterValue` engine config (kag.parser.js `makeTag`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum KeepSpaceLevel {
    /// Level 1: strip every half-width space from the value while reading it
    /// (backquote-delimited values are exempt). Reproduces the legacy
    /// pre-V514 parser behaviour.
    RemoveAll,
    /// Level 2 (engine default, also used when the config is undefined):
    /// keep interior spaces but trim both ends of the final value.
    #[default]
    TrimEnds,
    /// Level 3: keep the value exactly as written, no trimming.
    KeepAll,
}

/// Configuration for the TyranoScript parser.
///
/// The boolean flags below reproduce quirks of the reference implementation
/// (tyrano/plugins/kag/kag.parser.js). They default to the engine-compatible
/// behaviour; switch them off to get the stricter / lossless interpretation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParserConfig {
    /// How whitespace inside parameter values is treated. Maps to the
    /// engine's `KeepSpaceInParameterValue` config value 1/2/3.
    pub keep_space_in_parameter_value: KeepSpaceLevel,

    /// 悪法 (engine quirk, default ON for compatibility):
    /// `parseScenario` leaves script mode for ANY line that merely
    /// *contains* the substring `endscript` — including string literals such
    /// as `var s = "endscript";` — and then parses that line as ordinary
    /// scenario content. Set to `false` to require a real
    /// `[endscript]` / `@endscript` line instead.
    pub loose_endscript_termination: bool,

    /// 悪法 (engine quirk, default ON for compatibility):
    /// for `*label|value|extra` the engine takes `split("|")[1]`, silently
    /// dropping `|extra`. Set to `false` to keep the full remainder
    /// (`value|extra`) as the label value.
    pub label_value_first_segment_only: bool,

    /// 悪法 (engine quirk, default ON for compatibility):
    /// for `#name:face:extra` the engine takes `split(":")[1]` as the face,
    /// silently dropping `:extra`. Set to `false` to keep the full remainder
    /// (`face:extra`) as the face.
    pub chara_face_first_segment_only: bool,
}

impl Default for ParserConfig {
    fn default() -> Self {
        ParserConfig {
            keep_space_in_parameter_value: KeepSpaceLevel::default(),
            loose_endscript_termination: true,
            label_value_first_segment_only: true,
            chara_face_first_segment_only: true,
        }
    }
}

impl ParserConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_keep_space(mut self, level: KeepSpaceLevel) -> Self {
        self.keep_space_in_parameter_value = level;
        self
    }
}
