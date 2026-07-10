//! Parameter and tag schemas, plus the registry lookup.

use std::collections::HashMap;
use std::sync::LazyLock;

use super::builtin::BUILTIN_TAGS;
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
    /// Builds a registry over `specs`. Panics on duplicate tag names —
    /// the table is static data, so that is a programming error.
    pub fn new(specs: &'static [TagSpec]) -> TagRegistry {
        let mut by_name = HashMap::with_capacity(specs.len());
        for spec in specs {
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

/// The registry over [`BUILTIN_TAGS`].
pub fn builtin_registry() -> &'static TagRegistry {
    static REGISTRY: LazyLock<TagRegistry> = LazyLock::new(|| TagRegistry::new(BUILTIN_TAGS));
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
    fn seed_set_is_present() {
        let reg = builtin_registry();
        assert!(reg.len() >= 30, "seed table has ~30 tags, got {}", reg.len());
        for name in ["jump", "call", "macro", "endmacro", "if", "endif", "l", "p", "s", "eval"] {
            assert!(reg.get(name).is_some(), "`{name}` missing from builtin table");
        }
        for spec in BUILTIN_TAGS {
            assert!(!spec.doc.is_empty(), "`{}` has no doc line", spec.name);
        }
    }
}
