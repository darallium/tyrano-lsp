//! Pure value-kind rules: no database, exhaustively unit-testable.
//!
//! Two functions split the problem exactly where staticness ends:
//! [`classify`] decides *whether* a parameter value can be checked at
//! all, [`check_static`] applies one rule per [`ValueKind`] variant to
//! the values that can. Dynamic values (`&expr` entities, `%param` refs)
//! satisfy every kind by construction (gradual typing — see
//! `tyrano_project::registry::kind`).

use tyrano_syntax::SyntaxKind;
use tyrano_syntax::ast::{InterpretOptions, Param};
use tyrano_project::ValueKind;

/// How a parameter's written value relates to static checking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueClass {
    /// A statically-known (cooked) value: check it.
    Static(String),
    /// An `&expr` entity: evaluated at runtime, satisfies every kind.
    Entity,
    /// A `%param` forward inside a macro body: dynamic, satisfies every
    /// kind.
    ParamRef,
    /// No value to speak of: a bare flag (`foo` without `=`) or the
    /// macro `*` pass-through.
    Absent,
}

/// Classifies one parameter's value.
pub fn classify(param: &Param, opts: &InterpretOptions) -> ValueClass {
    if param.is_macro_star() || !param.has_eq() {
        return ValueClass::Absent;
    }
    if let Some(value) = param.value_node()
        && let Some(token) = value.token()
    {
        match token.kind() {
            SyntaxKind::ENTITY => return ValueClass::Entity,
            SyntaxKind::PARAM_REF => return ValueClass::ParamRef,
            _ => {}
        }
    }
    // `name=` with no value node cooks to the empty string.
    ValueClass::Static(param.cooked_value(opts).unwrap_or_default())
}

/// A static value failing its expected kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindMismatch {
    pub expected: ValueKind,
    /// One clause describing the expectation, for the diagnostic message.
    pub expectation: String,
}

fn mismatch(expected: ValueKind, expectation: impl Into<String>) -> Result<(), KindMismatch> {
    Err(KindMismatch { expected, expectation: expectation.into() })
}

/// Checks a cooked static value against `kind`. One rule per variant:
///
/// - `Label`: `*` followed by a non-empty name
/// - `Number`: an integer or float literal
/// - `Boolean`: `true` | `false`
/// - `Color`: `#` or `0x` followed by 6 or 8 hex digits
/// - `Enum`: membership in the word list
/// - `VariableName`: `f`/`sf`/`tf`/`mp` then `.segment`+
/// - `Any` / `Text` / `Expression` / `Scenario` / `Asset`: always pass
///   (for the last two, *shape* always passes — existence is the
///   cross-file checker's job, not a kind rule)
pub fn check_static(kind: ValueKind, raw: &str) -> Result<(), KindMismatch> {
    match kind {
        ValueKind::Any
        | ValueKind::Text
        | ValueKind::Expression
        | ValueKind::Scenario
        | ValueKind::Asset(_) => Ok(()),
        ValueKind::Label => match raw.strip_prefix('*') {
            Some(name) if !name.is_empty() => Ok(()),
            _ => mismatch(kind, "a `*label` reference"),
        },
        ValueKind::Number => {
            if raw.parse::<i64>().is_ok() || raw.parse::<f64>().is_ok() {
                Ok(())
            } else {
                mismatch(kind, "a number")
            }
        }
        ValueKind::Boolean => {
            if raw == "true" || raw == "false" {
                Ok(())
            } else {
                mismatch(kind, "`true` or `false`")
            }
        }
        ValueKind::Color => {
            let digits = raw.strip_prefix('#').or_else(|| {
                raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X"))
            });
            match digits {
                Some(d)
                    if (d.len() == 6 || d.len() == 8)
                        && d.chars().all(|c| c.is_ascii_hexdigit()) =>
                {
                    Ok(())
                }
                _ => mismatch(kind, "a `#rrggbb[aa]` or `0xrrggbb[aa]` color"),
            }
        }
        ValueKind::Enum(words) => {
            if words.contains(&raw) {
                Ok(())
            } else {
                mismatch(kind, format!("one of {}", words.join(", ")))
            }
        }
        ValueKind::VariableName => {
            let mut segments = raw.split('.');
            let root = segments.next().unwrap_or("");
            let mut rest = segments.peekable();
            if matches!(root, "f" | "sf" | "tf" | "mp")
                && rest.peek().is_some()
                && rest.all(|s| !s.is_empty())
            {
                Ok(())
            } else {
                mismatch(kind, "a game-variable path like `f.name`")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_project::AssetKind;
    use tyrano_syntax::ast::{AnyTag, Line, Scenario};

    /// Parses a one-tag line and returns its params.
    fn params_of(line: &str) -> Vec<Param> {
        let parse = tyrano_syntax::parse(line);
        let scenario = Scenario::from_parse(&parse);
        let tag = scenario
            .lines()
            .find_map(|l| match l {
                Line::AtTag(t) => Some(AnyTag::At(t)),
                Line::Text(t) => t.segments().into_iter().find_map(|s| match s {
                    tyrano_syntax::ast::TextSegment::Tag(t) => Some(AnyTag::Inline(t)),
                    _ => None,
                }),
                _ => None,
            })
            .expect("fixture line contains a tag");
        tag.params()
    }

    #[test]
    fn classify_covers_every_class() {
        let opts = InterpretOptions::default();
        // `empty=` must come last: the engine-compatible lexer would
        // otherwise consume the following word as its (space-led) value.
        let params = params_of("[m * flag plain=x ent=&f.x fwd=%val empty=]\n");
        let classes: Vec<ValueClass> =
            params.iter().map(|p| classify(p, &opts)).collect();
        assert_eq!(
            classes,
            [
                ValueClass::Absent, // *
                ValueClass::Absent, // bare flag
                ValueClass::Static("x".to_string()),
                ValueClass::Entity,
                ValueClass::ParamRef,
                ValueClass::Static(String::new()),
            ]
        );
    }

    #[test]
    fn classify_cooks_quotes_and_spaces() {
        let opts = InterpretOptions::default();
        let params = params_of("[m a=\"hello world\" b= 1 ]\n");
        assert_eq!(classify(&params[0], &opts), ValueClass::Static("hello world".to_string()));
    }

    fn ok(kind: ValueKind, raw: &str) {
        assert_eq!(check_static(kind, raw), Ok(()), "{kind:?} should accept {raw:?}");
    }

    fn bad(kind: ValueKind, raw: &str) {
        assert!(check_static(kind, raw).is_err(), "{kind:?} should reject {raw:?}");
    }

    #[test]
    fn label_rule() {
        ok(ValueKind::Label, "*start");
        bad(ValueKind::Label, "start");
        bad(ValueKind::Label, "*");
        bad(ValueKind::Label, "");
    }

    #[test]
    fn number_rule() {
        for raw in ["0", "42", "-3", "3.5", "-0.25", "1e3"] {
            ok(ValueKind::Number, raw);
        }
        for raw in ["", "abc", "1px", "3,5"] {
            bad(ValueKind::Number, raw);
        }
    }

    #[test]
    fn boolean_rule() {
        ok(ValueKind::Boolean, "true");
        ok(ValueKind::Boolean, "false");
        for raw in ["True", "1", "yes", ""] {
            bad(ValueKind::Boolean, raw);
        }
    }

    #[test]
    fn color_rule() {
        for raw in ["#ff0000", "0xff0000", "0XFF0000", "#ff0000cc"] {
            ok(ValueKind::Color, raw);
        }
        for raw in ["ff0000", "#ff000", "#ff00001", "#gg0000", "0x12345", "red", ""] {
            bad(ValueKind::Color, raw);
        }
    }

    #[test]
    fn enum_rule() {
        const WORDS: &[&str] = &["left", "right"];
        ok(ValueKind::Enum(WORDS), "left");
        bad(ValueKind::Enum(WORDS), "center");
        let err = check_static(ValueKind::Enum(WORDS), "center").unwrap_err();
        assert_eq!(err.expectation, "one of left, right");
    }

    #[test]
    fn variable_name_rule() {
        for raw in ["f.hp", "sf.flags.cleared", "tf.x", "mp.text"] {
            ok(ValueKind::VariableName, raw);
        }
        for raw in ["f", "f.", "g.hp", "f..hp", "hp", ""] {
            bad(ValueKind::VariableName, raw);
        }
    }

    #[test]
    fn pass_through_kinds_accept_anything() {
        for kind in [
            ValueKind::Any,
            ValueKind::Text,
            ValueKind::Expression,
            ValueKind::Scenario,
            ValueKind::Asset(AssetKind::BgImage),
        ] {
            ok(kind, "");
            ok(kind, "whatever && f.x > 3");
        }
    }
}
