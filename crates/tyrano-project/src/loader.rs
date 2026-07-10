//! File-system loading, quarantined from all query code.
//!
//! [`load_project`] walks a project root and produces pure
//! [`ProjectMetadata`]; only [`crate::ProjectDatabase::new`] turns that
//! into salsa inputs. Nothing else in this crate touches `std::fs`.

use std::io;
use std::path::Path;

use crate::path::ProjectPath;
use crate::project::{ProjectMetadata, ProjectSettings};

/// Reads the project at `root` (the directory containing `data/`) using
/// `settings` to decide which directories to scan.
///
/// Scenario roots contribute `.ks` files (recursively) with their text;
/// asset roots contribute every file's path. Missing root directories are
/// fine — a project without e.g. `data/video` simply has no videos. Files
/// whose paths do not normalize (or scenario files that are not UTF-8)
/// are reported as errors rather than silently skipped.
pub fn load_project_with(root: &Path, settings: ProjectSettings) -> io::Result<ProjectMetadata> {
    let mut metadata =
        ProjectMetadata { settings, scenario_sources: Vec::new(), assets: Default::default() };

    for scenario_root in metadata.settings.scenario_roots.clone() {
        for rel in walk_sorted(root, &scenario_root)? {
            if rel.extension() != Some("ks") {
                continue;
            }
            let text = std::fs::read_to_string(root.join(rel.as_str()))?;
            metadata.scenario_sources.push((rel, text));
        }
    }

    for (kind, roots) in metadata.settings.asset_roots.clone() {
        for asset_root in roots {
            for rel in walk_sorted(root, &asset_root)? {
                metadata.assets.entry(kind).or_default().insert(rel);
            }
        }
    }

    Ok(metadata)
}

/// [`load_project_with`] under [`ProjectSettings::default`] (the standard
/// TyranoScript layout).
pub fn load_project(root: &Path) -> io::Result<ProjectMetadata> {
    load_project_with(root, ProjectSettings::default())
}

/// All regular files under `root/dir`, as sorted project-relative paths.
/// A missing directory yields no entries.
fn walk_sorted(root: &Path, dir: &ProjectPath) -> io::Result<Vec<ProjectPath>> {
    let mut out = Vec::new();
    let mut pending = vec![dir.clone()];
    while let Some(current) = pending.pop() {
        let abs = root.join(current.as_str());
        if !abs.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&abs)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("non-UTF-8 file name under {current}: {name:?}"),
                )
            })?;
            let rel = current
                .join(name)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            if entry.file_type()?.is_dir() {
                pending.push(rel);
            } else {
                out.push(rel);
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::AssetKind;
    use crate::{FileStatus, ProjectDatabase, ProjectDb as _};

    /// A self-cleaning unique temp directory (no external tempdir crate).
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> TempDir {
            let dir = std::env::temp_dir().join(format!(
                "tyrano-project-loader-{}-{:?}",
                std::process::id(),
                std::thread::current().id(),
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn write(&self, rel: &str, text: &str) {
            let abs = self.0.join(rel);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(abs, text).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn loads_standard_layout() {
        let tmp = TempDir::new();
        tmp.write("data/scenario/first.ks", "*start\n");
        tmp.write("data/scenario/sub/ev.ks", "*ev\n");
        tmp.write("data/scenario/readme.txt", "not a scenario");
        tmp.write("data/bgimage/room.jpg", "");
        tmp.write("data/bgm/theme.ogg", "");

        let metadata = load_project(&tmp.0).unwrap();
        let scenario_paths: Vec<&str> =
            metadata.scenario_sources.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            scenario_paths,
            ["data/scenario/first.ks", "data/scenario/sub/ev.ks"],
            "recursive, sorted, .ks only"
        );
        assert_eq!(metadata.scenario_sources[0].1, "*start\n");

        let bg: Vec<&str> =
            metadata.assets[&AssetKind::BgImage].iter().map(|p| p.as_str()).collect();
        assert_eq!(bg, ["data/bgimage/room.jpg"]);
        let bgm: Vec<&str> = metadata.assets[&AssetKind::Bgm].iter().map(|p| p.as_str()).collect();
        assert_eq!(bgm, ["data/bgm/theme.ogg"]);
        assert!(!metadata.assets.contains_key(&AssetKind::Video), "missing dirs load as empty");
    }

    #[test]
    fn loaded_metadata_builds_a_database() {
        let tmp = TempDir::new();
        tmp.write("data/scenario/a.ks", "*a\n[jump target=*a]\n");

        let db = ProjectDatabase::new(load_project(&tmp.0).unwrap());
        let files = db.project().scenario_files(&db);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status(&db), FileStatus::Exists);
        let idx = tyrano_parser_core::semantic_index(&db, files[0].source(&db));
        assert!(idx.label("a").is_some());
        assert!(idx.errors().is_empty());
    }
}
