use oxc_span::Span;

/// JavaScript source accepted by the embedded analyzer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    /// An `[iscript]` body or loaded `.js` file.
    Script,
    /// An `exp=`, `cond=`, or `&entity` snippet.
    Expression,
}

/// A byte range relative to the analyzed JavaScript region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextRange {
    pub start: u32,
    pub end: u32,
}

impl From<Span> for TextRange {
    fn from(span: Span) -> Self {
        Self {
            start: span.start,
            end: span.end,
        }
    }
}

/// A TyranoScript-owned variable namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EngineNamespace {
    F,
    Sf,
    Tf,
    Mp,
}

/// The binding at the root of a JavaScript variable path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VariableRoot {
    /// A canonical TyranoScript namespace, after alias normalization.
    Engine(EngineNamespace),
    /// A JavaScript lexical binding, identified by its definition site.
    Lexical { name: String, definition: TextRange },
    /// An unresolved JavaScript global other than an engine namespace.
    Global(String),
}

/// One property-selection step.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PathSegment {
    Static(String),
    /// A computed property whose precise value is not statically known.
    Dynamic(TextRange),
}

/// An identifier or property path to which an effect applies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VariablePath {
    pub root: VariableRoot,
    pub segments: Vec<PathSegment>,
}

/// JavaScript operation performed on a variable path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EffectKind {
    Define,
    Read,
    Write,
    Execute,
}

/// Additional semantics carried by an effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EffectRole {
    /// The path's value is consumed by the JavaScript operation.
    Value,
    /// The path must be non-nullish so a later property can be accessed.
    Dereference,
    /// A Write produced by `delete`; the target becomes undefined.
    Delete,
}

/// One arena-independent JavaScript variable effect.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JsEffect {
    pub kind: EffectKind,
    pub role: EffectRole,
    pub path: VariablePath,
    /// The complete JavaScript operation this effect was lowered from.
    pub range: TextRange,
    /// The identifier/property token representing `path` at the source site.
    /// This is kept separate from `range` so IDE navigation can distinguish
    /// `f`, `player`, and `hp` in `f.player.hp` without changing effect
    /// evaluation or diagnostic ranges.
    pub selection: TextRange,
    /// True for effects inside a function/arrow body. Such effects exist in
    /// the program but do not run merely because the containing file loads.
    pub deferred: bool,
    /// True when enclosing control flow may skip this effect.
    pub conditional: bool,
    /// Relative evaluation order. Writes/calls use the enclosing operation's
    /// end so RHS/argument effects precede them.
    pub order: u32,
}

/// One resolved JavaScript lexical-binding occurrence used by IDE features.
///
/// This stays separate from [`JsEffect`]: an immutable alias such as
/// `const state = sf.game` must normalize effects through `state`, while Go
/// Definition on the spelling `state` must still target its lexical binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JsBindingOccurrence {
    pub name: String,
    pub definition: TextRange,
    pub selection: TextRange,
    pub declaration: bool,
}

/// JavaScript runtime types represented by the abstract value engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum JsValueType {
    Undefined,
    Null,
    Boolean,
    Number,
    String,
    Array,
    Object,
    Function,
}

/// A compact union of possible JavaScript runtime types.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JsTypeSet(u16);

impl JsTypeSet {
    pub const NONE: Self = Self(0);
    pub const UNDEFINED: Self = Self(1 << 0);
    pub const NULL: Self = Self(1 << 1);
    pub const BOOLEAN: Self = Self(1 << 2);
    pub const NUMBER: Self = Self(1 << 3);
    pub const STRING: Self = Self(1 << 4);
    pub const ARRAY: Self = Self(1 << 5);
    pub const OBJECT: Self = Self(1 << 6);
    pub const FUNCTION: Self = Self(1 << 7);
    pub const UNKNOWN: Self = Self((1 << 8) - 1);

    pub const fn of(kind: JsValueType) -> Self {
        match kind {
            JsValueType::Undefined => Self::UNDEFINED,
            JsValueType::Null => Self::NULL,
            JsValueType::Boolean => Self::BOOLEAN,
            JsValueType::Number => Self::NUMBER,
            JsValueType::String => Self::STRING,
            JsValueType::Array => Self::ARRAY,
            JsValueType::Object => Self::OBJECT,
            JsValueType::Function => Self::FUNCTION,
        }
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Arena-independent abstract syntax for a JavaScript value.
///
/// This is intentionally an expression graph, not just a final constant:
/// semantic analysis can substitute `f`/`sf`/`tf`/`mp` paths using the
/// project flow state before enumerating value candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsValueExpr {
    Unknown,
    Undefined,
    Null,
    Boolean(bool),
    /// The original numeric spelling when available.
    Number(String),
    String(String),
    Path(VariablePath),
    /// JavaScript `+`, retaining both numeric-addition and concatenation
    /// semantics until operand types become known.
    Add(Box<JsValueExpr>, Box<JsValueExpr>),
    /// Alternatives from conditionals and short-circuit expressions.
    Choice(Vec<JsValueExpr>),
    Array(Vec<JsValueExpr>),
    Object(Vec<(String, JsValueExpr)>),
    Function,
    /// A value whose exact candidate is unknown but whose runtime type is.
    Typed(JsTypeSet),
}

impl JsValueExpr {
    pub fn possible_types(&self) -> JsTypeSet {
        match self {
            JsValueExpr::Unknown => JsTypeSet::UNKNOWN,
            JsValueExpr::Undefined => JsTypeSet::UNDEFINED,
            JsValueExpr::Null => JsTypeSet::NULL,
            JsValueExpr::Boolean(_) => JsTypeSet::BOOLEAN,
            JsValueExpr::Number(_) => JsTypeSet::NUMBER,
            JsValueExpr::String(_) => JsTypeSet::STRING,
            JsValueExpr::Path(_) => JsTypeSet::UNKNOWN,
            JsValueExpr::Add(left, right) => {
                let left = left.possible_types();
                let right = right.possible_types();
                let mut result = JsTypeSet::NONE;
                if left.intersects(JsTypeSet::STRING) || right.intersects(JsTypeSet::STRING) {
                    result = result.union(JsTypeSet::STRING);
                }
                if left.intersects(JsTypeSet::NUMBER) && right.intersects(JsTypeSet::NUMBER) {
                    result = result.union(JsTypeSet::NUMBER);
                }
                if left == JsTypeSet::UNKNOWN || right == JsTypeSet::UNKNOWN {
                    result = result.union(JsTypeSet::STRING).union(JsTypeSet::NUMBER);
                }
                if result.is_empty() {
                    JsTypeSet::UNKNOWN
                } else {
                    result
                }
            }
            JsValueExpr::Choice(values) => values.iter().fold(JsTypeSet::NONE, |types, value| {
                types.union(value.possible_types())
            }),
            JsValueExpr::Array(_) => JsTypeSet::ARRAY,
            JsValueExpr::Object(_) => JsTypeSet::OBJECT,
            JsValueExpr::Function => JsTypeSet::FUNCTION,
            JsValueExpr::Typed(types) => *types,
        }
    }
}

/// A JavaScript assignment with a structured right-hand-side value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsAssignment {
    pub target: VariablePath,
    pub value: JsValueExpr,
    pub range: TextRange,
    pub deferred: bool,
    pub conditional: bool,
    pub order: u32,
}

/// A type requirement imposed on a variable path by a JavaScript operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsPathConstraint {
    pub path: VariablePath,
    pub expected: JsTypeSet,
    pub range: TextRange,
}

/// A statically known equality test, retained for consumers that model
/// control-flow-sensitive APIs (for example, macro dispatch on `mp.type`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsStaticEquality {
    pub path: VariablePath,
    pub value: String,
    pub range: TextRange,
}

/// Arena-independent facts obtained from one JavaScript source region.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnedJsFacts {
    /// Whether Oxc accepted the complete region without a parse/semantic error.
    pub parsed: bool,
    /// Number of AST nodes visited while lowering the region.
    pub node_count: u32,
    /// Variable effects, including lexical definitions and normalized engine accesses.
    pub effects: Vec<JsEffect>,
    /// Resolved lexical declarations and references for source navigation.
    pub bindings: Vec<JsBindingOccurrence>,
    /// Structured assignments in source evaluation order.
    pub assignments: Vec<JsAssignment>,
    /// Value of a region analyzed as [`SourceKind::Expression`].
    pub result: Option<JsValueExpr>,
    /// Type constraints learned from structured JavaScript operations.
    pub constraints: Vec<JsPathConstraint>,
    /// Exact string equality tests found in the region.
    pub equalities: Vec<JsStaticEquality>,
    /// Parse/semantic diagnostic messages copied out of Oxc.
    pub diagnostics: Vec<String>,
}
