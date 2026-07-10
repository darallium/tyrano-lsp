//! The `lsp-server` adapter: LSP requests in, [`crate::ide`] calls out.
//!
//! Single-threaded by design — every feature is a memoized salsa query,
//! so responses are fast and a request queue never builds up meaningful
//! latency at the project sizes TyranoScript reaches.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Result;
use lsp_server::{Connection, ErrorCode, Message, Request, Response};
use lsp_types::notification::{Notification as _, PublishDiagnostics};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, InitializeParams, Location, MarkupContent, MarkupKind, OneOf,
    PublishDiagnosticsParams, ReferenceParams, ServerCapabilities, SymbolKind as LspSymbolKind,
    TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use tyrano_project::File;
use tyrano_syntax::text::TextSize;

use crate::ide;
use crate::position::{position_to_offset, range_to_lsp};
use crate::session::Session;

/// Capabilities this server advertises.
pub fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(
                ["[", "@", "=", "*", "/", "\""].map(str::to_string).to_vec(),
            ),
            ..CompletionOptions::default()
        }),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        ..ServerCapabilities::default()
    }
}

/// Performs the initialize handshake and runs the main loop until exit.
pub fn run(connection: Connection) -> Result<()> {
    let (initialize_id, initialize_params) = connection.initialize_start()?;
    let params: InitializeParams = serde_json::from_value(initialize_params)?;

    let result = serde_json::json!({
        "capabilities": server_capabilities(),
        "serverInfo": { "name": "tyrano-lsp", "version": env!("CARGO_PKG_VERSION") },
    });
    connection.initialize_finish(initialize_id, result)?;

    let root = workspace_root(&params);
    let session = match &root {
        Some(root) => Session::open(root).unwrap_or_else(|_| Session::empty(root.clone())),
        None => Session::empty(std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
    };

    main_loop(connection, session)
}

/// The workspace root from initialize params (first workspace folder,
/// falling back to the deprecated `root_uri`).
fn workspace_root(params: &InitializeParams) -> Option<PathBuf> {
    if let Some(folders) = &params.workspace_folders {
        if let Some(folder) = folders.first() {
            if let Ok(path) = folder.uri.to_file_path() {
                return Some(path);
            }
        }
    }
    #[allow(deprecated)]
    params.root_uri.as_ref().and_then(|uri| uri.to_file_path().ok())
}

fn main_loop(connection: Connection, mut session: Session) -> Result<()> {
    let mut open_docs: HashSet<Url> = HashSet::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    return Ok(());
                }
                let response = handle_request(&session, request);
                connection.sender.send(Message::Response(response))?;
            }
            Message::Notification(notification) => {
                handle_notification(&connection, &mut session, &mut open_docs, notification)?;
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
}

// ======================================================================
// Requests
// ======================================================================

fn handle_request(session: &Session, request: Request) -> Response {
    let id = request.id.clone();
    let result = match request.method.as_str() {
        "textDocument/hover" => with_params(request, |p: HoverParams| hover(session, p)),
        "textDocument/completion" => {
            with_params(request, |p: CompletionParams| completion(session, p))
        }
        "textDocument/definition" => {
            with_params(request, |p: GotoDefinitionParams| definition(session, p))
        }
        "textDocument/references" => {
            with_params(request, |p: ReferenceParams| references(session, p))
        }
        "textDocument/documentSymbol" => {
            with_params(request, |p: DocumentSymbolParams| document_symbols(session, p))
        }
        _ => {
            return Response::new_err(
                id,
                ErrorCode::MethodNotFound as i32,
                format!("unhandled method {}", request.method),
            );
        }
    };
    match result {
        Ok(value) => Response::new_ok(id, value),
        Err(err) => Response::new_err(id, ErrorCode::InvalidParams as i32, err.to_string()),
    }
}

/// Deserializes params and serializes the handler's result.
fn with_params<P, R>(
    request: Request,
    handler: impl FnOnce(P) -> R,
) -> Result<serde_json::Value>
where
    P: serde::de::DeserializeOwned,
    R: serde::Serialize,
{
    let params: P = serde_json::from_value(request.params)?;
    Ok(serde_json::to_value(handler(params))?)
}

/// Resolves a text-document position to `(file, offset)`.
fn resolve_position(
    session: &Session,
    uri: &Url,
    position: lsp_types::Position,
) -> Option<(File, TextSize)> {
    let abs = uri.to_file_path().ok()?;
    let file = session.file_at(&abs)?;
    let db = session.db();
    let source = file.source(db);
    let offset = position_to_offset(source.text(db), &tyrano_db::line_index(db, source), position)?;
    Some((file, offset))
}

/// Converts a [`ide::NavTarget`] to an LSP location.
fn to_location(session: &Session, target: &ide::NavTarget) -> Option<Location> {
    let abs = session.abs_path(&target.path);
    let uri = Url::from_file_path(&abs).ok()?;
    let db = session.db();
    let range = match db.file(&target.path) {
        Some(file) => {
            let source = file.source(db);
            range_to_lsp(source.text(db), &tyrano_db::line_index(db, source), target.range)
        }
        // Not a loaded file (an asset): point at the top.
        None => lsp_types::Range::default(),
    };
    Some(Location { uri, range })
}

fn hover(session: &Session, params: HoverParams) -> Option<Hover> {
    let position = params.text_document_position_params;
    let (file, offset) = resolve_position(session, &position.text_document.uri, position.position)?;
    let db = session.db();
    let result = ide::hover(db, file, offset)?;
    let source = file.source(db);
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: result.markdown,
        }),
        range: Some(range_to_lsp(
            source.text(db),
            &tyrano_db::line_index(db, source),
            result.range,
        )),
    })
}

fn completion(session: &Session, params: CompletionParams) -> Option<CompletionResponse> {
    let position = params.text_document_position;
    let (file, offset) = resolve_position(session, &position.text_document.uri, position.position)?;
    let items: Vec<CompletionItem> = ide::completions(session.db(), file, offset)
        .into_iter()
        .map(|item| CompletionItem {
            label: item.label,
            kind: Some(match item.kind {
                ide::CompletionKind::Tag => CompletionItemKind::KEYWORD,
                ide::CompletionKind::Macro => CompletionItemKind::FUNCTION,
                ide::CompletionKind::Label => CompletionItemKind::REFERENCE,
                ide::CompletionKind::File => CompletionItemKind::FILE,
                ide::CompletionKind::Asset => CompletionItemKind::FILE,
                ide::CompletionKind::Param => CompletionItemKind::PROPERTY,
                ide::CompletionKind::Value => CompletionItemKind::VALUE,
            }),
            detail: item.detail,
            ..CompletionItem::default()
        })
        .collect();
    Some(CompletionResponse::Array(items))
}

fn definition(session: &Session, params: GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
    let position = params.text_document_position_params;
    let (file, offset) = resolve_position(session, &position.text_document.uri, position.position)?;
    let locations: Vec<Location> = ide::goto_definition(session.db(), file, offset)
        .iter()
        .filter_map(|target| to_location(session, target))
        .collect();
    Some(GotoDefinitionResponse::Array(locations))
}

fn references(session: &Session, params: ReferenceParams) -> Option<Vec<Location>> {
    let position = params.text_document_position;
    let (file, offset) = resolve_position(session, &position.text_document.uri, position.position)?;
    let locations: Vec<Location> = ide::references(
        session.db(),
        file,
        offset,
        params.context.include_declaration,
    )
    .iter()
    .filter_map(|target| to_location(session, target))
    .collect();
    Some(locations)
}

fn document_symbols(
    session: &Session,
    params: DocumentSymbolParams,
) -> Option<DocumentSymbolResponse> {
    let abs = params.text_document.uri.to_file_path().ok()?;
    let file = session.file_at(&abs)?;
    let db = session.db();
    let source = file.source(db);
    let text = source.text(db);
    let index = tyrano_db::line_index(db, source);

    let symbols: Vec<DocumentSymbol> = ide::document_symbols(db, file)
        .into_iter()
        .map(|symbol| {
            #[allow(deprecated)]
            DocumentSymbol {
                name: symbol.name,
                detail: None,
                kind: match symbol.kind {
                    tyrano_parser_core::SymbolKind::Label => LspSymbolKind::CONSTANT,
                    tyrano_parser_core::SymbolKind::Macro => LspSymbolKind::FUNCTION,
                    tyrano_parser_core::SymbolKind::Character => LspSymbolKind::OBJECT,
                },
                tags: None,
                deprecated: None,
                range: range_to_lsp(text, &index, symbol.full_range),
                selection_range: range_to_lsp(text, &index, symbol.range),
                children: None,
            }
        })
        .collect();
    Some(DocumentSymbolResponse::Nested(symbols))
}

// ======================================================================
// Notifications
// ======================================================================

fn handle_notification(
    connection: &Connection,
    session: &mut Session,
    open_docs: &mut HashSet<Url>,
    notification: lsp_server::Notification,
) -> Result<()> {
    use lsp_types::notification::{
        DidChangeTextDocument, DidChangeWatchedFiles, DidCloseTextDocument, DidOpenTextDocument,
    };

    match notification.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: lsp_types::DidOpenTextDocumentParams =
                serde_json::from_value(notification.params)?;
            let uri = params.text_document.uri;
            if let Ok(abs) = uri.to_file_path() {
                session.set_text(&abs, params.text_document.text);
            }
            open_docs.insert(uri);
            publish_all(connection, session, open_docs)?;
        }
        DidChangeTextDocument::METHOD => {
            let mut params: lsp_types::DidChangeTextDocumentParams =
                serde_json::from_value(notification.params)?;
            // Full sync: the last change carries the whole text.
            if let Some(change) = params.content_changes.pop() {
                if let Ok(abs) = params.text_document.uri.to_file_path() {
                    session.set_text(&abs, change.text);
                }
            }
            publish_all(connection, session, open_docs)?;
        }
        DidCloseTextDocument::METHOD => {
            let params: lsp_types::DidCloseTextDocumentParams =
                serde_json::from_value(notification.params)?;
            if let Ok(abs) = params.text_document.uri.to_file_path() {
                session.revert_to_disk(&abs);
            }
            // Clear diagnostics for the closed document.
            open_docs.remove(&params.text_document.uri);
            send_diagnostics(connection, params.text_document.uri, Vec::new())?;
            publish_all(connection, session, open_docs)?;
        }
        DidChangeWatchedFiles::METHOD => {
            let params: lsp_types::DidChangeWatchedFilesParams =
                serde_json::from_value(notification.params)?;
            for event in params.changes {
                let Ok(abs) = event.uri.to_file_path() else { continue };
                // Open documents are owned by the editor, not the disk.
                if open_docs.contains(&event.uri) {
                    continue;
                }
                let exists = event.typ != lsp_types::FileChangeType::DELETED;
                session.sync_from_disk(&abs, exists);
            }
            publish_all(connection, session, open_docs)?;
        }
        _ => {}
    }
    Ok(())
}

/// Publishes diagnostics for every open document. Cheap: each file's
/// check is a memoized query, so unchanged files cost a lookup.
fn publish_all(
    connection: &Connection,
    session: &Session,
    open_docs: &HashSet<Url>,
) -> Result<()> {
    for uri in open_docs {
        let diagnostics = uri
            .to_file_path()
            .ok()
            .and_then(|abs| session.file_at(&abs))
            .map(|file| file_diagnostics(session, file))
            .unwrap_or_default();
        send_diagnostics(connection, uri.clone(), diagnostics)?;
    }
    Ok(())
}

fn file_diagnostics(session: &Session, file: File) -> Vec<Diagnostic> {
    let db = session.db();
    let source = file.source(db);
    let text = source.text(db);
    let index = tyrano_db::line_index(db, source);
    tyrano_semantic::check_file(db, file)
        .iter()
        .map(|d| Diagnostic {
            range: range_to_lsp(text, &index, d.range),
            severity: Some(match d.severity {
                tyrano_syntax::diagnostics::Severity::Error => DiagnosticSeverity::ERROR,
                tyrano_syntax::diagnostics::Severity::Warning => DiagnosticSeverity::WARNING,
                tyrano_syntax::diagnostics::Severity::Info => DiagnosticSeverity::INFORMATION,
            }),
            code: Some(lsp_types::NumberOrString::String(d.code.to_string())),
            source: Some("tyrano".to_string()),
            message: d.message.clone(),
            ..Diagnostic::default()
        })
        .collect()
}

fn send_diagnostics(
    connection: &Connection,
    uri: Url,
    diagnostics: Vec<Diagnostic>,
) -> Result<()> {
    let params = PublishDiagnosticsParams { uri, diagnostics, version: None };
    connection.sender.send(Message::Notification(lsp_server::Notification {
        method: PublishDiagnostics::METHOD.to_string(),
        params: serde_json::to_value(params)?,
    }))?;
    Ok(())
}
