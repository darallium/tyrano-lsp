//! Normalized project-relative paths.
//!
//! A [`ProjectPath`] is the project layer's file identity: always relative
//! to the project root, always forward-slash separated, never containing
//! `.`/`..`/empty segments. Normalizing at construction time means path
//! comparison and map lookup are plain string operations everywhere else.

use std::fmt;

/// A normalized project-relative path (`"data/scenario/first.ks"`).
///
/// Construction rejects absolute paths and `..` segments; `\` separators
/// and redundant `.`/empty segments are normalized away.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProjectPath(String);

/// Why a string was rejected as a [`ProjectPath`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectPathError {
    /// Absolute (`/…`) or drive-qualified (`C:…`) paths cannot be
    /// project-relative.
    Absolute(String),
    /// `..` segments would escape the project root.
    ParentSegment(String),
    /// Nothing left after normalization (empty string, `"."`, `"//"`, …).
    Empty,
}

impl fmt::Display for ProjectPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectPathError::Absolute(p) => write!(f, "absolute path not allowed: {p:?}"),
            ProjectPathError::ParentSegment(p) => {
                write!(f, "`..` segment not allowed: {p:?}")
            }
            ProjectPathError::Empty => write!(f, "empty path"),
        }
    }
}

impl std::error::Error for ProjectPathError {}

impl ProjectPath {
    /// Normalizes `raw` into a project-relative path.
    pub fn new(raw: impl AsRef<str>) -> Result<ProjectPath, ProjectPathError> {
        let raw = raw.as_ref();
        let unified = raw.replace('\\', "/");
        if unified.starts_with('/') {
            return Err(ProjectPathError::Absolute(raw.to_string()));
        }
        let segments: Vec<&str> =
            unified.split('/').filter(|s| !s.is_empty() && *s != ".").collect();
        if segments.first().is_some_and(|s| s.contains(':')) {
            return Err(ProjectPathError::Absolute(raw.to_string()));
        }
        if segments.contains(&"..") {
            return Err(ProjectPathError::ParentSegment(raw.to_string()));
        }
        if segments.is_empty() {
            return Err(ProjectPathError::Empty);
        }
        Ok(ProjectPath(segments.join("/")))
    }

    /// The normalized path text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `self` extended by `tail` (which is normalized by the same rules).
    pub fn join(&self, tail: &str) -> Result<ProjectPath, ProjectPathError> {
        ProjectPath::new(format!("{}/{}", self.0, tail))
    }

    /// The extension of the final segment (without the dot), if any.
    pub fn extension(&self) -> Option<&str> {
        let name = self.0.rsplit('/').next()?;
        let (stem, ext) = name.rsplit_once('.')?;
        (!stem.is_empty() && !ext.is_empty()).then_some(ext)
    }

    /// The path remainder under directory `prefix`, or `None` when `self`
    /// is not strictly inside it.
    pub fn strip_prefix(&self, prefix: &ProjectPath) -> Option<&str> {
        let rest = self.0.strip_prefix(&prefix.0)?;
        rest.strip_prefix('/')
    }
}

impl fmt::Display for ProjectPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_separators_and_dot_segments() {
        let p = ProjectPath::new("data\\scenario\\.\\first.ks").unwrap();
        assert_eq!(p.as_str(), "data/scenario/first.ks");
        assert_eq!(ProjectPath::new("a//b/").unwrap().as_str(), "a/b");
    }

    #[test]
    fn rejects_absolute_parent_and_empty() {
        assert!(matches!(ProjectPath::new("/etc/passwd"), Err(ProjectPathError::Absolute(_))));
        assert!(matches!(ProjectPath::new("C:\\game\\a.ks"), Err(ProjectPathError::Absolute(_))));
        assert!(matches!(ProjectPath::new("a/../b"), Err(ProjectPathError::ParentSegment(_))));
        assert!(matches!(ProjectPath::new(""), Err(ProjectPathError::Empty)));
        assert!(matches!(ProjectPath::new("./."), Err(ProjectPathError::Empty)));
    }

    #[test]
    fn join_and_extension() {
        let root = ProjectPath::new("data/scenario").unwrap();
        let file = root.join("sub/ev.ks").unwrap();
        assert_eq!(file.as_str(), "data/scenario/sub/ev.ks");
        assert_eq!(file.extension(), Some("ks"));
        assert_eq!(root.extension(), None);
        assert_eq!(ProjectPath::new("a/.hidden").unwrap().extension(), None);
        assert!(root.join("../escape").is_err());
    }

    #[test]
    fn strip_prefix_requires_segment_boundary() {
        let root = ProjectPath::new("data/scenario").unwrap();
        let file = ProjectPath::new("data/scenario/first.ks").unwrap();
        assert_eq!(file.strip_prefix(&root), Some("first.ks"));
        let outside = ProjectPath::new("data/scenario2/first.ks").unwrap();
        assert_eq!(outside.strip_prefix(&root), None);
        assert_eq!(root.strip_prefix(&root), None, "a directory is not inside itself");
    }

    #[test]
    fn ordering_is_by_normalized_text() {
        let a = ProjectPath::new("a/b").unwrap();
        let b = ProjectPath::new("a\\c").unwrap();
        assert!(a < b);
    }
}
