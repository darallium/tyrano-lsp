//! Pure IDE feature functions.
//!
//! Every feature is a plain function over `&dyn ProjectDb` + a byte
//! offset, returning protocol-free domain types ([`NavTarget`],
//! [`HoverResult`], …). The `server` module translates these to LSP
//! shapes; tests exercise them directly on in-memory projects.

mod completion;
mod cursor;
mod goto;
mod hover;
mod references;
mod symbols;

pub use completion::{CompletionItem, CompletionKind, completions};
pub use cursor::{CursorTarget, classify};
pub use goto::goto_definition;
pub use hover::{HoverResult, hover};
pub use references::references;
pub use symbols::{DocSymbol, document_symbols};

use tyrano_project::ProjectPath;
use tyrano_syntax::text::TextRange;

/// A place in the project to navigate to (project-relative, so the
/// adapter can point at asset files that are not loaded [`File`]s too).
///
/// [`File`]: tyrano_project::File
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavTarget {
    pub path: ProjectPath,
    /// Byte range in the target file (empty range at 0 for whole-file
    /// targets such as `storage=` jumps and assets).
    pub range: TextRange,
}

#[cfg(test)]
pub(crate) mod testutil {
    use tyrano_project::testing::ProjectBuilder;
    use tyrano_project::{File, ProjectDatabase, ProjectPath};
    use tyrano_syntax::text::TextSize;

    /// Builds an in-memory project from `(path, text)` pairs.
    pub fn project(files: &[(&str, &str)]) -> ProjectDatabase {
        let mut builder = ProjectBuilder::new();
        for (path, text) in files {
            builder = builder.file(path, text);
        }
        builder.build()
    }

    pub fn file(db: &ProjectDatabase, path: &str) -> File {
        db.file(&ProjectPath::new(path).unwrap()).expect("fixture file exists")
    }

    /// Byte offset of the first occurrence of `needle` in `file`'s text,
    /// plus `add` bytes.
    pub fn offset(db: &ProjectDatabase, file: File, needle: &str, add: u32) -> TextSize {
        let text = file.source(db).text(db);
        let at = text.find(needle).unwrap_or_else(|| panic!("needle {needle:?} in {text:?}"));
        TextSize::new(at as u32 + add)
    }
}
