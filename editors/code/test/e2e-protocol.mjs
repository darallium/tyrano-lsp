#!/usr/bin/env node
// Layer 1 end-to-end test: speaks raw LSP JSON-RPC (Content-Length framing)
// against the real tyrano-lsp binary, using editors/code/testdata as workspace.
//
// Run with: node test/e2e-protocol.mjs   (npm run test:e2e)
//
// Dependency-free: hand-rolled framing over the child process stdio pipes.

import { spawn } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";
import { readFileSync } from "node:fs";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const serverPath = path.resolve(__dirname, "../../../target/release/tyrano-lsp");
const workspaceRoot = path.resolve(__dirname, "../testdata");
const scenarioDir = path.join(workspaceRoot, "data", "scenario");
const firstPath = path.join(scenarioDir, "first.ks");
const scene2Path = path.join(scenarioDir, "scene2.ks");

const firstUri = pathToFileURL(firstPath).toString();
const scene2Uri = pathToFileURL(scene2Path).toString();
const workspaceUri = pathToFileURL(workspaceRoot).toString();

const firstText = readFileSync(firstPath, "utf8");
const scene2Text = readFileSync(scene2Path, "utf8");

// ---- assertion helpers ---------------------------------------------------

let passed = 0;
function ok(cond, message) {
  if (!cond) {
    fail(message);
  }
  passed += 1;
  console.log(`  ok - ${message}`);
}
function fail(message) {
  console.error(`\nFAILED: ${message}`);
  cleanupAndExit(1);
}
function cleanupAndExit(code) {
  try {
    child.kill("SIGKILL");
  } catch {
    /* ignore */
  }
  process.exit(code);
}

// ---- position math -------------------------------------------------------

// Find the 0-based line index containing `needle`, and the UTF-16 character
// offset of `needle` within that line. Positions in LSP are UTF-16 code units;
// JavaScript strings are already UTF-16, so `indexOf` gives the right offset.
function locate(text, needle, occurrence = 0) {
  const lines = text.split("\n");
  let seen = 0;
  for (let line = 0; line < lines.length; line++) {
    let from = 0;
    for (;;) {
      const idx = lines[line].indexOf(needle, from);
      if (idx === -1) break;
      if (seen === occurrence) {
        return { line, character: idx };
      }
      seen += 1;
      from = idx + needle.length;
    }
  }
  throw new Error(`could not locate ${JSON.stringify(needle)} in text`);
}

// Position on the label name inside `target=*top` of first.ks (on the "top").
const jumpTop = locate(firstText, "target=*top");
const hoverPos = { line: jumpTop.line, character: jumpTop.character + "target=*".length + 1 };

// The *top label definition in scene2.ks is on line 0; put cursor on "top".
const topDef = locate(scene2Text, "*top");
const topDefPos = { line: topDef.line, character: topDef.character + 1 };

// ---- LSP client over stdio ----------------------------------------------

const child = spawn(serverPath, [], { stdio: ["pipe", "pipe", "pipe"] });

child.on("error", (err) => fail(`failed to spawn server: ${String(err)}`));

let exitInfo = null;
child.on("exit", (code, signal) => {
  exitInfo = { code, signal };
});

let stderrBuf = "";
child.stderr.on("data", (d) => {
  stderrBuf += d.toString();
});

let nextId = 1;
const pendingResponses = new Map(); // id -> resolve
const notificationBuffers = new Map(); // method -> array of params
const notificationWaiters = new Map(); // method -> array of {predicate, resolve}

let recvBuf = Buffer.alloc(0);
child.stdout.on("data", (chunk) => {
  recvBuf = Buffer.concat([recvBuf, chunk]);
  for (;;) {
    const headerEnd = recvBuf.indexOf("\r\n\r\n");
    if (headerEnd === -1) return;
    const header = recvBuf.slice(0, headerEnd).toString("ascii");
    const m = /Content-Length:\s*(\d+)/i.exec(header);
    if (!m) {
      fail(`missing Content-Length in header: ${JSON.stringify(header)}`);
    }
    const len = parseInt(m[1], 10);
    const bodyStart = headerEnd + 4;
    if (recvBuf.length < bodyStart + len) return;
    const body = recvBuf.slice(bodyStart, bodyStart + len).toString("utf8");
    recvBuf = recvBuf.slice(bodyStart + len);
    let msg;
    try {
      msg = JSON.parse(body);
    } catch (e) {
      fail(`invalid JSON from server: ${String(e)}: ${body}`);
    }
    dispatch(msg);
  }
});

function dispatch(msg) {
  if (msg.id !== undefined && (msg.result !== undefined || msg.error !== undefined)) {
    const resolve = pendingResponses.get(msg.id);
    if (resolve) {
      pendingResponses.delete(msg.id);
      resolve(msg);
    }
    return;
  }
  if (msg.method) {
    // Server -> client request or notification.
    if (msg.id !== undefined) {
      // Requests from server (e.g. workspace/configuration): reply null.
      send({ jsonrpc: "2.0", id: msg.id, result: null });
      return;
    }
    const arr = notificationBuffers.get(msg.method) ?? [];
    arr.push(msg.params);
    notificationBuffers.set(msg.method, arr);
    const waiters = notificationWaiters.get(msg.method) ?? [];
    const remaining = [];
    for (const w of waiters) {
      if (w.predicate(msg.params)) {
        w.resolve(msg.params);
      } else {
        remaining.push(w);
      }
    }
    notificationWaiters.set(msg.method, remaining);
  }
}

function send(obj) {
  const json = JSON.stringify(obj);
  const buf = Buffer.from(json, "utf8");
  child.stdin.write(`Content-Length: ${buf.length}\r\n\r\n`);
  child.stdin.write(buf);
}

function request(method, params) {
  const id = nextId++;
  return new Promise((resolve) => {
    pendingResponses.set(id, (msg) => {
      if (msg.error) {
        fail(`request ${method} returned error: ${JSON.stringify(msg.error)}`);
      }
      resolve(msg.result);
    });
    send({ jsonrpc: "2.0", id, method, params });
  });
}

function notify(method, params) {
  send({ jsonrpc: "2.0", method, params });
}

function waitForNotification(method, predicate = () => true, timeoutMs = 8000) {
  // Check already-buffered notifications first.
  const buffered = notificationBuffers.get(method) ?? [];
  for (const params of buffered) {
    if (predicate(params)) return Promise.resolve(params);
  }
  return new Promise((resolve, reject) => {
    const waiter = { predicate, resolve };
    const arr = notificationWaiters.get(method) ?? [];
    arr.push(waiter);
    notificationWaiters.set(method, arr);
    setTimeout(() => {
      const cur = notificationWaiters.get(method) ?? [];
      notificationWaiters.set(
        method,
        cur.filter((w) => w !== waiter)
      );
      reject(new Error(`timeout waiting for notification ${method}`));
    }, timeoutMs);
  });
}

function hoverToString(contents) {
  if (contents == null) return "";
  if (typeof contents === "string") return contents;
  if (Array.isArray(contents)) return contents.map(hoverToString).join("\n");
  if (typeof contents.value === "string") return contents.value; // MarkupContent / MarkedString
  return JSON.stringify(contents);
}

function uriEndsWith(uri, suffix) {
  return typeof uri === "string" && uri.endsWith(suffix);
}

// ---- test sequence -------------------------------------------------------

async function main() {
  console.log(`Server:    ${serverPath}`);
  console.log(`Workspace: ${workspaceRoot}`);
  console.log(`hover/definition position: line ${hoverPos.line} char ${hoverPos.character}`);
  console.log(`*top definition position:  line ${topDefPos.line} char ${topDefPos.character}`);

  // 1. initialize
  const initResult = await request("initialize", {
    processId: process.pid,
    clientInfo: { name: "e2e-protocol", version: "0.0.0" },
    rootUri: workspaceUri,
    capabilities: {
      textDocument: {
        hover: { contentFormat: ["markdown", "plaintext"] },
        completion: { completionItem: { snippetSupport: true } },
        publishDiagnostics: {},
      },
      workspace: { workspaceFolders: true },
    },
    workspaceFolders: [{ uri: workspaceUri, name: "testdata" }],
  });
  const caps = initResult.capabilities ?? {};
  ok(caps.hoverProvider === true || (caps.hoverProvider && typeof caps.hoverProvider === "object"),
    "initialize returns hoverProvider");
  notify("initialized", {});

  // 2. didOpen first.ks
  notify("textDocument/didOpen", {
    textDocument: { uri: firstUri, languageId: "tyranoscript", version: 1, text: firstText },
  });

  // 7 (part a). publishDiagnostics arrives for first.ks and is empty (clean).
  const diagFirst = await waitForNotification(
    "textDocument/publishDiagnostics",
    (p) => p.uri === firstUri
  );
  ok(Array.isArray(diagFirst.diagnostics), "publishDiagnostics arrives for first.ks");
  ok(diagFirst.diagnostics.length === 0,
    `first.ks diagnostics are empty (clean fixture) [got ${diagFirst.diagnostics.length}]`);

  // 2. hover over *top in the jump target -> mentions scene2.ks
  const hover = await request("textDocument/hover", {
    textDocument: { uri: firstUri },
    position: hoverPos,
  });
  const hoverText = hoverToString(hover && hover.contents);
  ok(hoverText.includes("scene2.ks"),
    `hover on *top jump target mentions scene2.ks [got: ${JSON.stringify(hoverText).slice(0, 200)}]`);

  // 3. definition at same position -> location in scene2.ks at line 0
  const defResult = await request("textDocument/definition", {
    textDocument: { uri: firstUri },
    position: hoverPos,
  });
  const defLoc = Array.isArray(defResult) ? defResult[0] : defResult;
  ok(defLoc != null, "definition returned a location");
  const defUri = defLoc.uri ?? defLoc.targetUri;
  const defRange = defLoc.range ?? defLoc.targetSelectionRange ?? defLoc.targetRange;
  ok(uriEndsWith(defUri, "scene2.ks"), `definition points into scene2.ks [got ${defUri}]`);
  ok(defRange.start.line === 0, `definition targets line 0 in scene2.ks [got ${defRange.start.line}]`);

  // 4. completion after typing "[" appended to first.ks
  const changedText = firstText + "[";
  notify("textDocument/didChange", {
    textDocument: { uri: firstUri, version: 2 },
    contentChanges: [{ text: changedText }], // full sync
  });
  const changedLines = changedText.split("\n");
  const complPos = { line: changedLines.length - 1, character: changedLines[changedLines.length - 1].length };
  const complResult = await request("textDocument/completion", {
    textDocument: { uri: firstUri },
    position: complPos,
    context: { triggerKind: 2, triggerCharacter: "[" },
  });
  const items = Array.isArray(complResult) ? complResult : (complResult && complResult.items) || [];
  const labels = items.map((it) => (typeof it.label === "string" ? it.label : it.label && it.label.label));
  ok(labels.includes("jump"), `completion includes "jump" [got ${labels.length} items]`);
  ok(labels.includes("greet"), `completion includes macro "greet"`);

  // Restore first.ks content so later state is clean.
  notify("textDocument/didChange", {
    textDocument: { uri: firstUri, version: 3 },
    contentChanges: [{ text: firstText }],
  });

  // 5. references on the *top label definition in scene2.ks
  notify("textDocument/didOpen", {
    textDocument: { uri: scene2Uri, languageId: "tyranoscript", version: 1, text: scene2Text },
  });
  await waitForNotification("textDocument/publishDiagnostics", (p) => p.uri === scene2Uri);
  const refs = await request("textDocument/references", {
    textDocument: { uri: scene2Uri },
    position: topDefPos,
    context: { includeDeclaration: true },
  });
  ok(Array.isArray(refs) && refs.length >= 2,
    `references on *top returns 2+ locations [got ${Array.isArray(refs) ? refs.length : "none"}]`);
  const refUris = new Set((refs || []).map((r) => r.uri));
  ok(refUris.has(scene2Uri) && refUris.has(firstUri),
    `references span both scene2.ks and first.ks [got ${[...refUris].join(", ")}]`);

  // 6. documentSymbol on first.ks -> label "start" and macro "greet"
  const symResult = await request("textDocument/documentSymbol", {
    textDocument: { uri: firstUri },
  });
  const symNames = collectSymbolNames(symResult);
  ok(symNames.some((n) => n === "start" || n === "*start" || n.includes("start")),
    `documentSymbol lists label "start" [got ${JSON.stringify(symNames)}]`);
  ok(symNames.some((n) => n === "greet" || n.includes("greet")),
    `documentSymbol lists macro "greet"`);

  // 8. shutdown / exit cleanly
  await request("shutdown", null);
  notify("exit", null);

  await new Promise((resolve) => {
    const start = Date.now();
    const t = setInterval(() => {
      if (exitInfo !== null || Date.now() - start > 5000) {
        clearInterval(t);
        resolve();
      }
    }, 20);
  });
  ok(exitInfo !== null, "server process exited after exit notification");
  ok(exitInfo.code === 0, `server exited with code 0 [got code=${exitInfo && exitInfo.code}, signal=${exitInfo && exitInfo.signal}]`);

  console.log(`\nAll ${passed} assertions passed.`);
  process.exit(0);
}

function collectSymbolNames(symbols) {
  const names = [];
  const visit = (arr) => {
    for (const s of arr || []) {
      if (s.name) names.push(s.name);
      if (s.children) visit(s.children);
    }
  };
  visit(symbols || []);
  return names;
}

main().catch((err) => {
  console.error(stderrBuf ? `\nserver stderr:\n${stderrBuf}` : "");
  fail(String(err && err.stack ? err.stack : err));
});
