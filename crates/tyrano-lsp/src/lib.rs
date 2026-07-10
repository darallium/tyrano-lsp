//! LSP layer for TyranoScript.
//!
//! Pipeline position: every other crate → **this crate**. Three layers,
//! separated for testability:
//!
//! - [`ide`]: pure feature functions (`hover`, `goto_definition`,
//!   `completion`, `references`, `document_symbols`) over `&ProjectDatabase`
//!   plus a byte offset — no protocol types, fully unit-testable.
//! - [`session`]: one open workspace — root discovery, URI ↔ project-path
//!   mapping, file synchronization into the salsa database.
//! - [`server`]: the `lsp-server` adapter translating LSP requests into
//!   [`ide`] calls and [`session`] mutations.

pub mod ide;
pub mod position;
pub mod server;
pub mod session;
