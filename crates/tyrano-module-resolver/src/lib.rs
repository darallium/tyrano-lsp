//! `storage=` resolution: from the raw string a script writes to a
//! project [`File`], and back.
//!
//! Pipeline position: `tyrano-project` (files, project inputs) → **this
//! crate** → `tyrano-semantic` (cross-file checks). The file-local layer
//! records `Resolution::External { storage, .. }` without interpreting
//! it; this crate gives that string a meaning against the project's
//! scenario roots.

mod module;
mod resolver;
mod storage_name;

pub use module::{ScriptModule, script_module};
pub use resolver::{StorageResolver, candidate_paths, resolve_storage};
pub use storage_name::StorageName;

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_project::testing::ProjectBuilder;
    use tyrano_project::{FileChange, ProjectDb as _, ProjectPath, ProjectSettings};

    fn path(s: &str) -> ProjectPath {
        ProjectPath::new(s).unwrap()
    }

    #[test]
    fn candidate_paths_complete_the_extension() {
        let settings = ProjectSettings::default();
        let cands: Vec<String> = candidate_paths(&settings, "scene2")
            .into_iter()
            .map(|p| p.as_str().to_string())
            .collect();
        assert_eq!(cands, ["data/scenario/scene2", "data/scenario/scene2.ks"]);

        let cands: Vec<String> = candidate_paths(&settings, "sub/ev.ks")
            .into_iter()
            .map(|p| p.as_str().to_string())
            .collect();
        assert_eq!(cands, ["data/scenario/sub/ev.ks"], "explicit extension: no completion");

        assert!(candidate_paths(&settings, "../escape.ks").is_empty(), "bad names: no candidates");
    }

    #[test]
    fn resolves_existing_storage() {
        let db = ProjectBuilder::new()
            .file("data/scenario/first.ks", "*start\n")
            .file("data/scenario/sub/ev.ks", "*ev\n")
            .build();
        let resolver = StorageResolver::new(&db);

        let first = db.file(&path("data/scenario/first.ks")).unwrap();
        assert_eq!(resolver.resolve("first.ks"), Some(first));
        assert_eq!(resolver.resolve("first"), Some(first), "extension completed");
        assert_eq!(
            resolver.resolve("sub/ev"),
            db.file(&path("data/scenario/sub/ev.ks")),
            "subdirectories work"
        );
        assert_eq!(resolver.resolve("nope.ks"), None);
    }

    #[test]
    fn first_scenario_root_wins() {
        let settings = ProjectSettings {
            scenario_roots: vec![path("mods/scenario"), path("data/scenario")],
            ..ProjectSettings::default()
        };
        let db = ProjectBuilder::new()
            .settings(settings)
            .file("mods/scenario/a.ks", "*mod\n")
            .file("data/scenario/a.ks", "*base\n")
            .build();

        let resolved = StorageResolver::new(&db).resolve("a.ks").unwrap();
        assert_eq!(resolved.path(&db).as_str(), "mods/scenario/a.ks");
    }

    #[test]
    fn missing_storage_resolves_after_creation() {
        let mut db = ProjectBuilder::new().file("data/scenario/first.ks", "*start\n").build();
        let name = StorageName::new(&db, "scene2.ks".to_string());
        assert_eq!(resolve_storage(&db, name), None);

        db.apply_file_change(
            &path("data/scenario/scene2.ks"),
            FileChange::Created("*s2\n".to_string()),
        );
        let resolved = resolve_storage(&db, name).expect("created file now resolves");
        assert_eq!(resolved.path(&db).as_str(), "data/scenario/scene2.ks");

        // And back to None on deletion.
        db.apply_file_change(&path("data/scenario/scene2.ks"), FileChange::Deleted);
        assert_eq!(resolve_storage(&db, name), None);
    }

    #[test]
    fn script_module_roundtrips_through_resolve_storage() {
        let db = ProjectBuilder::new()
            .file("data/scenario/first.ks", "*start\n")
            .file("data/scenario/sub/ev.ks", "*ev\n")
            .build();

        for file in db.project().scenario_files(&db).clone() {
            let module = script_module(&db, file).expect("scenario files have modules");
            assert_eq!(
                resolve_storage(&db, module.storage),
                Some(file),
                "storage name {:?} must resolve back to its file",
                module.storage.text(&db),
            );
        }
        let ev = db.file(&path("data/scenario/sub/ev.ks")).unwrap();
        let module = script_module(&db, ev).unwrap();
        assert_eq!(module.storage.text(&db), "sub/ev.ks", "canonical storage keeps subdirs");
    }

    #[test]
    fn unrooted_file_has_no_module() {
        let mut db = ProjectBuilder::new().build();
        db.apply_file_change(&path("elsewhere/x.ks"), FileChange::Created("*x\n".to_string()));
        let stray = db.file(&path("elsewhere/x.ks")).unwrap();
        assert_eq!(script_module(&db, stray), None);
    }
}
