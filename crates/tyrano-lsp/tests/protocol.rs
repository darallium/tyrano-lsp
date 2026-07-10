//! In-process LSP protocol tests: a client on the other end of
//! `Connection::memory()` drives the full server loop against a real
//! on-disk fixture project.

use std::path::PathBuf;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::Notification as _;
use serde_json::{Value, json};

/// A self-cleaning fixture project + connected client endpoint.
struct TestServer {
    client: Connection,
    root: PathBuf,
    server: Option<std::thread::JoinHandle<anyhow::Result<()>>>,
    next_id: i32,
}

impl TestServer {
    /// Writes `files` under a fresh project root, boots the server, and
    /// completes the initialize handshake.
    fn start(tag: &str, files: &[(&str, &str)]) -> TestServer {
        let root = std::env::temp_dir().join(format!(
            "tyrano-lsp-proto-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        let _ = std::fs::remove_dir_all(&root);
        for (rel, text) in files {
            let abs = root.join(rel);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(abs, text).unwrap();
        }

        let (server_conn, client) = Connection::memory();
        let server = std::thread::spawn(move || tyrano_lsp::server::run(server_conn));
        let mut this = TestServer { client, root, server: Some(server), next_id: 0 };

        let root_uri = lsp_types::Url::from_file_path(&this.root).unwrap();
        let init = this.request(
            "initialize",
            json!({
                "capabilities": {},
                "workspaceFolders": [{ "uri": root_uri, "name": "fixture" }],
            }),
        );
        assert!(init["capabilities"]["hoverProvider"].as_bool().unwrap());
        this.notify("initialized", json!({}));
        this
    }

    fn uri(&self, rel: &str) -> lsp_types::Url {
        lsp_types::Url::from_file_path(self.root.join(rel)).unwrap()
    }

    fn notify(&self, method: &str, params: Value) {
        self.client
            .sender
            .send(Message::Notification(Notification {
                method: method.to_string(),
                params,
            }))
            .unwrap();
    }

    /// Sends a request and pumps messages until its response arrives.
    /// Server-initiated notifications seen along the way are dropped.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        self.client
            .sender
            .send(Message::Request(Request { id: id.clone(), method: method.to_string(), params }))
            .unwrap();
        loop {
            match self.recv() {
                Message::Response(Response { id: got, result, error, .. }) if got == id => {
                    assert!(error.is_none(), "error response: {error:?}");
                    return result.unwrap_or(Value::Null);
                }
                _ => {}
            }
        }
    }

    /// Pumps messages until a `textDocument/publishDiagnostics` for `uri`
    /// arrives, returning its diagnostics array.
    fn wait_diagnostics(&self, uri: &lsp_types::Url) -> Vec<Value> {
        loop {
            if let Message::Notification(n) = self.recv() {
                if n.method == lsp_types::notification::PublishDiagnostics::METHOD
                    && n.params["uri"] == json!(uri)
                {
                    return n.params["diagnostics"].as_array().cloned().unwrap_or_default();
                }
            }
        }
    }

    fn recv(&self) -> Message {
        self.client
            .receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("server answered within 10s")
    }

    fn open(&self, rel: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": self.uri(rel), "languageId": "tyranoscript",
                    "version": 1, "text": text,
                }
            }),
        );
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        let _ = self.client.sender.send(Message::Request(Request {
            id,
            method: "shutdown".to_string(),
            params: json!(null),
        }));
        let _ = self
            .client
            .sender
            .send(Message::Notification(Notification { method: "exit".to_string(), params: json!(null) }));
        if let Some(handle) = self.server.take() {
            handle.join().expect("server thread").expect("server loop exits cleanly");
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const MAIN: &str = "\
*start
[jump storage=scene2.ks target=*top]
[greet]
";
const SCENE2: &str = "\
*top
[macro name=greet]hello[endmacro]
";

fn fixture(tag: &str) -> TestServer {
    TestServer::start(
        tag,
        &[
            ("data/scenario/main.ks", MAIN),
            ("data/scenario/scene2.ks", SCENE2),
        ],
    )
}

fn doc_position(server: &TestServer, rel: &str, line: u32, character: u32) -> Value {
    json!({
        "textDocument": { "uri": server.uri(rel) },
        "position": { "line": line, "character": character },
    })
}

#[test]
fn hover_across_files_over_protocol() {
    let mut server = fixture("hover");
    server.open("data/scenario/main.ks", MAIN);
    // Line 1, inside `*top` (cross-file jump target).
    let hover = server.request(
        "textDocument/hover",
        doc_position(&server, "data/scenario/main.ks", 1, 32),
    );
    let text = hover["contents"]["value"].as_str().expect("markdown hover");
    assert!(text.contains("scene2.ks"), "hover names the target file: {text}");
}

#[test]
fn goto_definition_lands_in_other_file() {
    let mut server = fixture("goto");
    server.open("data/scenario/main.ks", MAIN);
    let response = server.request(
        "textDocument/definition",
        doc_position(&server, "data/scenario/main.ks", 1, 32),
    );
    let locations = response.as_array().expect("array response");
    assert_eq!(locations.len(), 1, "{response}");
    assert_eq!(locations[0]["uri"], json!(server.uri("data/scenario/scene2.ks")));
    assert_eq!(locations[0]["range"]["start"], json!({ "line": 0, "character": 1 }));
}

#[test]
fn completion_offers_cross_file_macro() {
    let mut server = fixture("completion");
    // The editor types a fresh `[` on line 3 of main.ks.
    let edited = format!("{MAIN}[");
    server.open("data/scenario/main.ks", MAIN);
    server.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": server.uri("data/scenario/main.ks"), "version": 2 },
            "contentChanges": [{ "text": edited }],
        }),
    );
    let response = server.request(
        "textDocument/completion",
        doc_position(&server, "data/scenario/main.ks", 3, 1),
    );
    let items = response.as_array().expect("array response");
    let labels: Vec<&str> = items.iter().filter_map(|i| i["label"].as_str()).collect();
    assert!(labels.contains(&"jump"), "builtin tags offered: {labels:?}");
    assert!(labels.contains(&"greet"), "cross-file macro offered: {labels:?}");
}

#[test]
fn references_span_the_project() {
    let mut server = fixture("references");
    server.open("data/scenario/scene2.ks", SCENE2);
    // On the `top` label definition (line 0, character 1).
    let response = server.request(
        "textDocument/references",
        json!({
            "textDocument": { "uri": server.uri("data/scenario/scene2.ks") },
            "position": { "line": 0, "character": 1 },
            "context": { "includeDeclaration": true },
        }),
    );
    let locations = response.as_array().expect("array response");
    assert_eq!(locations.len(), 2, "decl + cross-file use: {response}");
    assert!(
        locations.iter().any(|l| l["uri"] == json!(server.uri("data/scenario/main.ks"))),
        "{response}"
    );
}

#[test]
fn diagnostics_follow_edits_and_cross_file_changes() {
    let server = fixture("diagnostics");
    let main_uri = server.uri("data/scenario/main.ks");

    server.open("data/scenario/main.ks", MAIN);
    let clean = server.wait_diagnostics(&main_uri);
    assert_eq!(clean, Vec::<Value>::new(), "fixture is clean");

    // Break the cross-file target: scene2.ks loses `*top` on disk.
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({ "changes": [{ "uri": server.uri("data/scenario/scene2.ks"), "type": 3 }] }),
    );
    let broken = server.wait_diagnostics(&main_uri);
    assert!(
        broken
            .iter()
            .any(|d| d["message"].as_str().unwrap_or_default().contains("scene2.ks")),
        "cross-file breakage reported in main.ks: {broken:?}"
    );
}

#[test]
fn document_symbols_lists_labels_and_macros() {
    let mut server = fixture("symbols");
    server.open("data/scenario/scene2.ks", SCENE2);
    let response = server.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": server.uri("data/scenario/scene2.ks") } }),
    );
    let symbols = response.as_array().expect("array response");
    let names: Vec<&str> = symbols.iter().filter_map(|s| s["name"].as_str()).collect();
    assert_eq!(names, ["top", "greet"], "{response}");
}

#[test]
fn positions_use_utf16_columns() {
    let server_files: &[(&str, &str)] = &[(
        "data/scenario/jp.ks",
        "*開始\nこんにちは[jump target=*開始]\n",
    )];
    let mut server = TestServer::start("utf16", server_files);
    server.open("data/scenario/jp.ks", "*開始\nこんにちは[jump target=*開始]\n");
    // Line 1: こんにちは = 5 UTF-16 units, then `[jump target=*開始]`;
    // the `開` of the target sits at UTF-16 column 5+14 = 19... target=* is
    // 13 chars after `[jump ` (6). 5 + 1 + 5 + 1 + 7 + 1 = hover on col 20.
    let hover = server.request(
        "textDocument/hover",
        doc_position(&server, "data/scenario/jp.ks", 1, 20),
    );
    let text = hover["contents"]["value"].as_str().expect("markdown hover for 開始");
    assert!(text.contains("開始"), "{text}");
}
