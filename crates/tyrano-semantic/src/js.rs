//! JavaScript regions embedded in scenario files.
//!
//! Oxc is invoked only inside [`file_js_analysis`]. Its arena-backed values
//! are lowered immediately by `tyrano-js-analysis`; Salsa sees only this
//! module's owned, equality-comparable region records.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use tyrano_db::FileDiagnostic;
use tyrano_js_analysis::{
    EffectKind, OwnedJsFacts, PathSegment, SourceKind, VariableRoot, analyze,
};
use tyrano_project::{File, Project, ProjectDb, ProjectPath};
use tyrano_syntax::SyntaxKind;
use tyrano_syntax::ast::Param;
use tyrano_syntax::diagnostics::Severity;
use tyrano_syntax::text::{TextRange, TextSize};

use crate::check::all_tags;
use crate::js_navigation::JsSourceLocation;

/// Host syntax that supplied one JavaScript region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsRegionKind {
    JavaScriptFile,
    IScript,
    Parameter { tag: String, parameter: String },
    Entity { tag: String, parameter: String },
}

/// One analyzed region. Effect ranges inside `facts` are relative to `range`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsRegion {
    pub kind: JsRegionKind,
    pub range: TextRange,
    pub facts: OwnedJsFacts,
}

/// Complete JavaScript coverage for one scenario/library/JavaScript source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileJsAnalysis {
    pub regions: Vec<JsRegion>,
}

impl FileJsAnalysis {
    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    pub fn parsed_region_count(&self) -> usize {
        self.regions
            .iter()
            .filter(|region| region.facts.parsed)
            .count()
    }

    pub fn effect_count(&self) -> usize {
        self.regions
            .iter()
            .map(|region| region.facts.effects.len())
            .sum()
    }

    pub fn all_regions_parsed(&self) -> bool {
        self.parsed_region_count() == self.region_count()
    }
}

/// Analyzes every `[iscript]`, expression parameter, and `&entity` in `file`.
#[salsa::tracked]
pub fn file_js_analysis(db: &dyn ProjectDb, file: File) -> Arc<FileJsAnalysis> {
    let source_file = file.source(db);
    let source = source_file.text(db);
    if file.path(db).extension() == Some("js") {
        return Arc::new(FileJsAnalysis {
            regions: vec![JsRegion {
                kind: JsRegionKind::JavaScriptFile,
                range: TextRange::new(TextSize::new(0), TextSize::of(source)),
                facts: analyze(source, SourceKind::Script),
            }],
        });
    }
    let module = tyrano_db::parsed_module(db, source_file);
    let index = tyrano_parser_core::semantic_index(db, source_file);
    let mut regions = Vec::new();

    for embedded in index
        .embedded_regions()
        .iter()
        .filter(|region| region.lang == tyrano_parser_core::EmbeddedLang::IScript)
    {
        let range = embedded.code_range;
        let code = slice(source, range);
        regions.push(JsRegion {
            kind: JsRegionKind::IScript,
            range,
            facts: analyze(code, SourceKind::Script),
        });
    }

    for tag in all_tags(&module.scenario()) {
        let tag_name = tag.name();
        for param in tag.params() {
            let param_name = param.name();
            let Some((range, code, kind)) = parameter_region(&param, &tag_name, &param_name) else {
                continue;
            };
            regions.push(JsRegion {
                kind,
                range,
                facts: analyze(&code, SourceKind::Expression),
            });
        }
    }

    regions.sort_by_key(|region| (region.range.start(), region.range.end()));
    Arc::new(FileJsAnalysis { regions })
}

/// Eager traversal of every project source. Individual files remain
/// independently memoized; reachability is deliberately not a coverage gate.
pub fn analyze_project_javascript(db: &dyn ProjectDb) -> Vec<(File, Arc<FileJsAnalysis>)> {
    let project = db.project();
    let mut files = project.scenario_files(db).clone();
    files.extend(project.library_files(db));
    files.extend(project.javascript_files(db));
    files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
    files.dedup();
    files
        .into_iter()
        .map(|file| (file, file_js_analysis(db, file)))
        .collect()
}

/// One custom Tyrano tag registration and the JavaScript property that
/// supplies its implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsTagRegistration {
    pub name: String,
    pub location: JsSourceLocation,
}

/// Statically recognizable custom tag registrations in reachable plugin
/// JavaScript. Unlike the name-only projection, this retains source locations
/// for Go Definition and Find References.
#[salsa::tracked]
pub fn project_javascript_tag_registrations(
    db: &dyn ProjectDb,
    project: Project,
) -> Arc<[JsTagRegistration]> {
    let _ = project.javascript_files(db);
    let mut registrations = Vec::new();
    let mut files = project.scenario_files(db).clone();
    files.extend(reachable_auxiliary_sources(db));
    files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
    files.dedup();

    for file in files {
        let analysis = file_js_analysis(db, file);
        for region in &analysis.regions {
            let has_dynamic_registration = region.facts.effects.iter().any(|effect| {
                effect.kind == EffectKind::Write
                    && (effect.path.segments.iter().any(
                        |segment| matches!(segment, PathSegment::Static(name) if name == "master_tag"),
                    ) || matches!(&effect.path.root, VariableRoot::Lexical { name, .. } if name == "master_tag"))
                    && effect
                        .path
                        .segments
                        .last()
                        .is_some_and(|segment| matches!(segment, PathSegment::Dynamic(_)))
            });
            for effect in &region.facts.effects {
                let Some(name) = registered_tag_name(effect, has_dynamic_registration) else {
                    continue;
                };
                let selection = region
                    .facts
                    .effects
                    .iter()
                    .find(|candidate| {
                        candidate.range == effect.range
                            && candidate.path.root == effect.path.root
                            && candidate.path.segments.len() <= effect.path.segments.len()
                            && candidate.path.segments
                                == effect.path.segments[..candidate.path.segments.len()]
                            && matches!(
                                candidate.path.segments.last(),
                                Some(PathSegment::Static(candidate_name)) if candidate_name == &name
                            )
                    })
                    .map_or(effect.selection, |candidate| candidate.selection);
                registrations.push(JsTagRegistration {
                    name,
                    location: JsSourceLocation {
                        file,
                        range: TextRange::new(
                            region.range.start() + TextSize::new(selection.start),
                            region.range.start() + TextSize::new(selection.end),
                        ),
                        kind: effect.kind,
                        role: effect.role,
                    },
                });
            }
        }
    }
    registrations.sort_by(|left, right| {
        left.name.cmp(&right.name).then_with(|| {
            (
                left.location.file.path(db),
                left.location.range.start(),
                left.location.range.end(),
            )
                .cmp(&(
                    right.location.file.path(db),
                    right.location.range.start(),
                    right.location.range.end(),
                ))
        })
    });
    registrations.dedup_by(|left, right| {
        left.name == right.name
            && left.location.file == right.location.file
            && left.location.range == right.location.range
    });
    registrations.into()
}

/// Statically registered custom tag names in reachable plugin JavaScript.
#[salsa::tracked]
pub fn project_javascript_tag_names(db: &dyn ProjectDb, project: Project) -> Arc<[String]> {
    let mut names = project_javascript_tag_registrations(db, project)
        .iter()
        .map(|registration| registration.name.clone())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names.into()
}

/// Registration locations of one reachable plugin tag.
pub fn project_javascript_tag_definitions(
    db: &dyn ProjectDb,
    project: Project,
    name: &str,
) -> Vec<JsSourceLocation> {
    project_javascript_tag_registrations(db, project)
        .iter()
        .filter(|registration| registration.name == name)
        .map(|registration| registration.location.clone())
        .collect()
}

/// The registered plugin tag property selected at `offset`, if any.
pub fn javascript_tag_at(db: &dyn ProjectDb, file: File, offset: TextSize) -> Option<String> {
    project_javascript_tag_registrations(db, db.project())
        .iter()
        .find(|registration| {
            registration.location.file == file
                && registration.location.range.contains_inclusive(offset)
        })
        .map(|registration| registration.name.clone())
}

fn registered_tag_name(
    effect: &tyrano_js_analysis::JsEffect,
    has_dynamic_registration: bool,
) -> Option<String> {
    if effect.kind != EffectKind::Write || effect.role == tyrano_js_analysis::EffectRole::Delete {
        return None;
    }
    // Common plugin idiom: build `tag.foo`, then copy its keys into
    // `master_tag[tag_name]` in a loop.
    if has_dynamic_registration
        && matches!(&effect.path.root, VariableRoot::Lexical { name, .. } if name == "tag")
        && let Some(PathSegment::Static(name)) = effect.path.segments.first()
    {
        return Some(name.clone());
    }

    let VariableRoot::Global(root) = &effect.path.root else {
        return None;
    };
    let segments = effect
        .path
        .segments
        .iter()
        .map(|segment| match segment {
            PathSegment::Static(name) => Some(name.as_str()),
            PathSegment::Dynamic(_) => None,
        })
        .collect::<Option<Vec<_>>>()?;
    match (root.as_str(), segments.as_slice()) {
        ("TYRANO", ["kag", "ftag", "master_tag", name, ..])
        | ("TYRANO", ["kag", "tag", name, ..])
        | ("tyrano", ["plugin", "kag", "ftag", "master_tag", name, ..])
        | ("tyrano", ["plugin", "kag", "tag", name, ..]) => Some((*name).to_owned()),
        _ => None,
    }
}

/// Finds plugin entry points and standalone scripts referenced by static
/// `[plugin]`/`[loadjs]` tags. All scenario files are roots: event targets
/// and dynamic scenario storage therefore do not force path enumeration,
/// while unreferenced plugin samples stay outside the executable program.
fn reachable_auxiliary_sources(db: &dyn ProjectDb) -> Vec<File> {
    let project = db.project();
    let mut seen: HashSet<File> = project.scenario_files(db).iter().copied().collect();
    let mut queue: VecDeque<File> = project.scenario_files(db).iter().copied().collect();
    let mut auxiliary = Vec::new();

    while let Some(file) = queue.pop_front() {
        if file.path(db).extension() != Some("ks") {
            continue;
        }
        let source_file = file.source(db);
        let module = tyrano_db::parsed_module(db, source_file);
        let options = source_file.interpret_options(db);
        for tag in all_tags(&module.scenario()) {
            let path = match tag.name().as_str() {
                "plugin" => {
                    let Some(name) = static_parameter(&tag, "name", &options) else {
                        continue;
                    };
                    let storage = static_parameter(&tag, "storage", &options)
                        .unwrap_or_else(|| "init.ks".to_owned());
                    ProjectPath::new(format!("data/others/plugin/{name}/{storage}"))
                }
                "loadjs" => {
                    let Some(storage) = static_parameter(&tag, "storage", &options) else {
                        continue;
                    };
                    let normalized = storage.strip_prefix("./").unwrap_or(&storage);
                    if normalized.starts_with("data/") {
                        ProjectPath::new(normalized)
                    } else {
                        ProjectPath::new(format!("data/others/{normalized}"))
                    }
                }
                _ => continue,
            };
            let Ok(path) = path else { continue };
            let Some(target) = db.files().get(&path) else {
                continue;
            };
            if seen.insert(target) {
                auxiliary.push(target);
                queue.push_back(target);
            }
        }
    }
    auxiliary.sort_by(|a, b| a.path(db).cmp(b.path(db)));
    auxiliary
}

fn static_parameter(
    tag: &tyrano_syntax::ast::AnyTag,
    name: &str,
    options: &tyrano_syntax::ast::InterpretOptions,
) -> Option<String> {
    let value = tag.param(name)?.cooked_value(options)?;
    (!value.trim_start().starts_with(['&', '%'])).then_some(value)
}

pub(crate) fn javascript_diagnostics(db: &dyn ProjectDb, file: File) -> Vec<FileDiagnostic> {
    file_js_analysis(db, file)
        .regions
        .iter()
        .filter(|region| !region.facts.parsed)
        .map(|region| FileDiagnostic {
            code: "xsem-js-parse",
            severity: Severity::Error,
            range: region.range,
            message: format!(
                "embedded JavaScript could not be analyzed: {}",
                region
                    .facts
                    .diagnostics
                    .first()
                    .map(String::as_str)
                    .unwrap_or("unknown parser error")
            ),
        })
        .collect()
}

fn parameter_region(
    param: &Param,
    tag: &str,
    parameter: &str,
) -> Option<(TextRange, String, JsRegionKind)> {
    let token = param.value_node()?.token()?;
    let raw_range = token.text_range();
    let (interior, interior_range) = strip_delimiters(token.text(), raw_range);
    if matches!(parameter, "exp" | "cond" | "preexp") {
        return Some((
            interior_range,
            interior.to_owned(),
            JsRegionKind::Parameter {
                tag: tag.to_owned(),
                parameter: parameter.to_owned(),
            },
        ));
    }

    let entity = if token.kind() == SyntaxKind::ENTITY {
        token.text().strip_prefix('&').map(|code| {
            strip_dangling_quote(
                code,
                TextRange::new(raw_range.start() + TextSize::new(1), raw_range.end()),
            )
        })
    } else {
        interior.strip_prefix('&').map(|code| {
            (
                code,
                TextRange::new(
                    interior_range.start() + TextSize::new(1),
                    interior_range.end(),
                ),
            )
        })
    };
    entity.map(|(code, range)| {
        (
            range,
            code.to_owned(),
            JsRegionKind::Entity {
                tag: tag.to_owned(),
                parameter: parameter.to_owned(),
            },
        )
    })
}

/// Tyrano projects in the wild sometimes contain `=&tf.value"`: the
/// leading quote is absent, but the engine still treats the trailing quote
/// as host syntax rather than JavaScript. Recover only this unambiguous
/// ENTITY-token case; ordinary malformed JavaScript remains a hard coverage
/// failure.
fn strip_dangling_quote(code: &str, range: TextRange) -> (&str, TextRange) {
    let Some(quote) = code
        .chars()
        .last()
        .filter(|quote| matches!(quote, '\'' | '"' | '`'))
    else {
        return (code, range);
    };
    if code.starts_with(quote) {
        return (code, range);
    }
    let width = quote.len_utf8();
    (
        &code[..code.len() - width],
        TextRange::new(
            range.start(),
            range.end() - TextSize::new(u32::try_from(width).expect("quote width fits u32")),
        ),
    )
}

fn slice(source: &str, range: TextRange) -> &str {
    &source[usize::from(range.start())..usize::from(range.end())]
}

fn strip_delimiters(raw: &str, range: TextRange) -> (&str, TextRange) {
    let Some(first) = raw
        .chars()
        .next()
        .filter(|ch| matches!(ch, '\'' | '"' | '`'))
    else {
        return (raw, range);
    };
    let width = first.len_utf8();
    let mut end = raw.len();
    if raw[width..].ends_with(first) {
        end -= width;
    }
    let start_offset = u32::try_from(width).expect("quote width fits u32");
    let end_offset = u32::try_from(raw.len() - end).expect("quote width fits u32");
    (
        &raw[width..end],
        TextRange::new(
            range.start() + TextSize::new(start_offset),
            range.end() - TextSize::new(end_offset),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_project::ProjectPath;
    use tyrano_project::testing::ProjectBuilder;

    #[test]
    fn covers_script_expression_and_quoted_entity() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/a.ks",
                "[eval exp=\"f.a = 1; f.b = 2\"]\n[image storage=\"&sf.image\"]\n[iscript]\nconst s = TYRANO.kag.variable.sf; s.x++;\n[endscript]\n",
            )
            .build();
        let file = db
            .file(&ProjectPath::new("data/scenario/a.ks").unwrap())
            .unwrap();
        let analysis = file_js_analysis(&db, file);
        assert_eq!(analysis.region_count(), 3);
        assert!(analysis.all_regions_parsed());
        assert!(analysis.effect_count() > 3);
    }

    #[test]
    fn recovers_dangling_host_quote_after_entity() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/a.ks",
                "[font face=&tf.face size=&tf.size\"]\n",
            )
            .build();
        let file = db
            .file(&ProjectPath::new("data/scenario/a.ks").unwrap())
            .unwrap();
        let analysis = file_js_analysis(&db, file);

        assert_eq!(analysis.region_count(), 2);
        assert!(analysis.all_regions_parsed());
        assert!(analysis.regions.iter().all(|region| {
            let source = file.source(&db).text(&db);
            !slice(source, region.range).ends_with('"')
        }));
    }

    #[test]
    fn covers_standalone_javascript_file() {
        let db = ProjectBuilder::new()
            .javascript(
                "data/others/plugin/demo/main.js",
                "sf.plugin.loaded = true;",
            )
            .build();
        let file = db
            .file(&ProjectPath::new("data/others/plugin/demo/main.js").unwrap())
            .unwrap();
        let analysis = file_js_analysis(&db, file);
        assert_eq!(analysis.region_count(), 1);
        assert!(matches!(
            analysis.regions[0].kind,
            JsRegionKind::JavaScriptFile
        ));
        assert!(analysis.all_regions_parsed());
    }

    #[test]
    fn resolves_tags_registered_by_reachable_javascript() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[loadjs storage=demo.js]\n[custom_tag]\n",
            )
            .javascript(
                "data/others/demo.js",
                "TYRANO.kag.ftag.master_tag.custom_tag = {};\n",
            )
            .build();

        let names = project_javascript_tag_names(&db, db.project());
        assert_eq!(&*names, &["custom_tag"]);
        let file = db
            .file(&ProjectPath::new("data/scenario/first.ks").unwrap())
            .unwrap();
        assert!(matches!(
            crate::SemanticModel::new(&db, file).resolve_tag("custom_tag"),
            crate::TagResolution::Plugin
        ));
        let definitions = project_javascript_tag_definitions(&db, db.project(), "custom_tag");
        assert_eq!(definitions.len(), 1, "{definitions:#?}");
        let js = db
            .file(&ProjectPath::new("data/others/demo.js").unwrap())
            .unwrap();
        let source = js.source(&db).text(&db);
        assert_eq!(
            &source[usize::from(definitions[0].range.start())
                ..usize::from(definitions[0].range.end())],
            "custom_tag"
        );
        assert_eq!(
            javascript_tag_at(&db, js, definitions[0].range.start()),
            Some("custom_tag".to_owned())
        );
    }

    #[test]
    fn registration_location_tracks_tag_name_in_copy_loop_idiom() {
        let source = "var tag = {}; tag.bg_rule.start = function() {}; var master_tag = TYRANO.kag.ftag.master_tag; for (var name in tag) { master_tag[name] = tag[name]; }";
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[loadjs storage=demo.js]\n[bg_rule]\n",
            )
            .javascript("data/others/demo.js", source)
            .build();
        let definitions = project_javascript_tag_definitions(&db, db.project(), "bg_rule");
        assert_eq!(definitions.len(), 1, "{definitions:#?}");
        assert_eq!(
            &source[usize::from(definitions[0].range.start())
                ..usize::from(definitions[0].range.end())],
            "bg_rule"
        );
    }

    #[test]
    fn project_coverage_includes_unreachable_sources_without_enabling_their_tags() {
        let db = ProjectBuilder::new()
            .file("data/scenario/first.ks", "[eval exp='f.live = 1']\n")
            .library(
                "data/others/plugin/unused/sample.ks",
                "[eval exp='f.sample = 1']\n",
            )
            .javascript(
                "data/others/plugin/unused/sample.js",
                "TYRANO.kag.ftag.master_tag.unused_tag = {};\n",
            )
            .build();

        let analyses = analyze_project_javascript(&db);
        assert_eq!(analyses.len(), 3);
        assert!(
            analyses
                .iter()
                .all(|(_, analysis)| analysis.all_regions_parsed())
        );
        assert!(project_javascript_tag_names(&db, db.project()).is_empty());
    }
}
