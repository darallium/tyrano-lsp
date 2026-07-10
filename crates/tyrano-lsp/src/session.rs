//! One open workspace: root discovery, path mapping, file sync.
//!
//! The session owns the [`ProjectDatabase`] and is the only place that
//! mutates it. All paths crossing this boundary are absolute host paths;
//! internally everything is a [`ProjectPath`] relative to the project
//! root (the directory containing `data/`).

use std::io;
use std::path::{Path, PathBuf};

use tyrano_project::{File, FileChange, ProjectDatabase, ProjectPath, load_project};

/// One open TyranoScript workspace.
pub struct Session {
    root: PathBuf,
    db: ProjectDatabase,
}

impl Session {
    /// Opens the project under `workspace`, discovering the actual project
    /// root ([`find_project_root`]). Falls back to an empty project when
    /// nothing on disk looks like one.
    pub fn open(workspace: &Path) -> io::Result<Session> {
        let root = find_project_root(workspace);
        let metadata = load_project(&root)?;
        Ok(Session { root, db: ProjectDatabase::new(metadata) })
    }

    /// An empty session rooted at `root` (tests, projects yet to exist).
    pub fn empty(root: PathBuf) -> Session {
        Session { root, db: ProjectDatabase::empty() }
    }

    pub fn db(&self) -> &ProjectDatabase {
        &self.db
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The project-relative path for an absolute host path, if it lies
    /// under the project root.
    pub fn project_path(&self, abs: &Path) -> Option<ProjectPath> {
        let rel = abs.strip_prefix(&self.root).ok()?;
        ProjectPath::new(rel.to_str()?).ok()
    }

    /// The absolute host path of a project-relative path.
    pub fn abs_path(&self, path: &ProjectPath) -> PathBuf {
        self.root.join(path.as_str())
    }

    /// The known [`File`] for an absolute host path.
    pub fn file_at(&self, abs: &Path) -> Option<File> {
        self.db.file(&self.project_path(abs)?)
    }

    /// Applies editor-provided text (didOpen / didChange full sync).
    /// Returns the file, or `None` when the path is outside the project.
    pub fn set_text(&mut self, abs: &Path, text: String) -> Option<File> {
        let path = self.project_path(abs)?;
        self.db.apply_file_change(&path, FileChange::Modified(text));
        self.db.file(&path)
    }

    /// Reverts a file to its on-disk state (didClose), or marks it deleted
    /// when it no longer exists on disk.
    pub fn revert_to_disk(&mut self, abs: &Path) {
        let Some(path) = self.project_path(abs) else { return };
        let change = match std::fs::read_to_string(abs) {
            Ok(text) => FileChange::Modified(text),
            Err(_) => FileChange::Deleted,
        };
        self.db.apply_file_change(&path, change);
    }

    /// Applies one watched-file event by re-reading the disk. `exists`
    /// mirrors the event kind (created/changed vs deleted); the disk is
    /// re-checked, so a stale event degrades gracefully.
    pub fn sync_from_disk(&mut self, abs: &Path, exists: bool) {
        let Some(path) = self.project_path(abs) else { return };
        if !exists || !abs.exists() {
            self.db.apply_file_change(&path, FileChange::Deleted);
            return;
        }
        // Scenario files need their text; other files (assets) only their
        // existence.
        let text = if path.extension() == Some("ks") {
            std::fs::read_to_string(abs).unwrap_or_default()
        } else {
            String::new()
        };
        self.db.apply_file_change(&path, FileChange::Modified(text));
    }
}

/// The directory that actually contains `data/scenario`: `workspace`
/// itself, or its first child (sorted) that qualifies — VSCode users
/// routinely open the folder one level above the game.
pub fn find_project_root(workspace: &Path) -> PathBuf {
    let is_project = |dir: &Path| dir.join("data").join("scenario").is_dir();
    if is_project(workspace) {
        return workspace.to_path_buf();
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(workspace)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    children.sort();
    children
        .into_iter()
        .find(|child| is_project(child))
        .unwrap_or_else(|| workspace.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_project::ProjectDb as _;

    /// A self-cleaning unique temp directory.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let dir = std::env::temp_dir().join(format!(
                "tyrano-lsp-session-{tag}-{}-{:?}",
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
    fn opens_project_and_maps_paths() {
        let tmp = TempDir::new("open");
        tmp.write("data/scenario/a.ks", "*start\n");
        let session = Session::open(&tmp.0).unwrap();

        let abs = tmp.0.join("data/scenario/a.ks");
        let file = session.file_at(&abs).expect("loaded file");
        assert_eq!(file.source(session.db()).text(session.db()), "*start\n");
        assert_eq!(session.abs_path(&ProjectPath::new("data/scenario/a.ks").unwrap()), abs);
        assert_eq!(session.project_path(Path::new("/elsewhere/x.ks")), None);
    }

    #[test]
    fn discovers_root_one_level_down() {
        let tmp = TempDir::new("nested");
        tmp.write("game/data/scenario/a.ks", "*start\n");
        assert_eq!(find_project_root(&tmp.0), tmp.0.join("game"));

        let session = Session::open(&tmp.0).unwrap();
        assert!(session.file_at(&tmp.0.join("game/data/scenario/a.ks")).is_some());
    }

    #[test]
    fn editor_text_overrides_and_revert_restores() {
        let tmp = TempDir::new("edit");
        tmp.write("data/scenario/a.ks", "*disk\n");
        let mut session = Session::open(&tmp.0).unwrap();
        let abs = tmp.0.join("data/scenario/a.ks");

        let file = session.set_text(&abs, "*edited\n".to_string()).unwrap();
        assert_eq!(file.source(session.db()).text(session.db()), "*edited\n");

        session.revert_to_disk(&abs);
        assert_eq!(file.source(session.db()).text(session.db()), "*disk\n");
    }

    #[test]
    fn watched_create_and_delete_update_project() {
        let tmp = TempDir::new("watch");
        tmp.write("data/scenario/a.ks", "*a\n");
        let mut session = Session::open(&tmp.0).unwrap();

        tmp.write("data/scenario/new.ks", "*fresh\n");
        let abs = tmp.0.join("data/scenario/new.ks");
        session.sync_from_disk(&abs, true);
        let db = session.db();
        assert_eq!(db.project().scenario_files(db).len(), 2);

        std::fs::remove_file(&abs).unwrap();
        session.sync_from_disk(&abs, false);
        let db = session.db();
        assert_eq!(db.project().scenario_files(db).len(), 1);
    }
}
