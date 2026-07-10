// Layer 2 suite: runs inside the VS Code extension host (CommonJS module
// loaded by @vscode/test-electron). Exports run() returning a Promise that
// resolves on success and rejects on any failed assertion.

const path = require("path");
const vscode = require("vscode");

const EXTENSION_ID = "darallium.tyranoscript";

function assert(cond, message) {
  if (!cond) {
    throw new Error(`Assertion failed: ${message}`);
  }
  console.log(`  ok - ${message}`);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// Poll `fn` until it returns a truthy value or the timeout elapses.
async function poll(fn, { timeout = 30000, interval = 300, label = "condition" } = {}) {
  const start = Date.now();
  let last;
  for (;;) {
    try {
      last = await fn();
      if (last) return last;
    } catch (err) {
      last = err;
    }
    if (Date.now() - start > timeout) {
      throw new Error(`timeout waiting for ${label} (last=${JSON.stringify(last)})`);
    }
    await sleep(interval);
  }
}

// Locate a substring within a TextDocument, returning a Position at `offset`
// characters past the start of the match. UTF-16 aware via document APIs.
function locatePosition(doc, needle, offset = 0) {
  const text = doc.getText();
  const idx = text.indexOf(needle);
  if (idx === -1) throw new Error(`could not locate ${JSON.stringify(needle)} in document`);
  return doc.positionAt(idx + offset);
}

function hoverToString(hovers) {
  const parts = [];
  for (const h of hovers || []) {
    for (const c of h.contents || []) {
      if (typeof c === "string") parts.push(c);
      else if (c && typeof c.value === "string") parts.push(c.value);
    }
  }
  return parts.join("\n");
}

function completionLabels(list) {
  const items = (list && list.items) || [];
  return items.map((it) => (typeof it.label === "string" ? it.label : it.label && it.label.label));
}

function symbolNames(symbols) {
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

async function run() {
  const folders = vscode.workspace.workspaceFolders;
  if (!folders || folders.length === 0) {
    throw new Error("no workspace folder open");
  }
  const root = folders[0].uri.fsPath;
  const firstPath = path.join(root, "data", "scenario", "first.ks");
  const firstUri = vscode.Uri.file(firstPath);

  // Activate the extension.
  const ext = vscode.extensions.getExtension(EXTENSION_ID);
  assert(ext != null, `extension ${EXTENSION_ID} is present`);
  await ext.activate();

  // Open first.ks; opening a tyranoscript file triggers activation of the
  // language client (activationEvents: onLanguage:tyranoscript).
  const doc = await vscode.workspace.openTextDocument(firstUri);
  await vscode.window.showTextDocument(doc);

  // Position on the "top" label inside `target=*top` of the jump tag.
  const jumpIdx = doc.getText().indexOf("target=*top");
  const hoverPos = doc.positionAt(jumpIdx + "target=*".length + 1);
  console.log(`hover/definition position: line ${hoverPos.line} char ${hoverPos.character}`);

  // Hover -> mentions scene2.ks. Poll because the server may need a moment.
  await poll(
    async () => {
      const hovers = await vscode.commands.executeCommand(
        "vscode.executeHoverProvider",
        firstUri,
        hoverPos
      );
      const text = hoverToString(hovers);
      return text.includes("scene2.ks") ? text : null;
    },
    { label: "hover mentioning scene2.ks" }
  );
  assert(true, "hover on *top jump target mentions scene2.ks");

  // Definition -> location whose uri ends with scene2.ks.
  const defUri = await poll(
    async () => {
      const defs = await vscode.commands.executeCommand(
        "vscode.executeDefinitionProvider",
        firstUri,
        hoverPos
      );
      const arr = Array.isArray(defs) ? defs : defs ? [defs] : [];
      for (const d of arr) {
        const uri = d.uri || d.targetUri;
        if (uri && uri.fsPath.endsWith("scene2.ks")) return uri;
      }
      return null;
    },
    { label: "definition into scene2.ks" }
  );
  assert(defUri.fsPath.endsWith("scene2.ks"), "definition resolves into scene2.ks");

  // Document symbols -> "start" and "greet".
  await poll(
    async () => {
      const symbols = await vscode.commands.executeCommand(
        "vscode.executeDocumentSymbolProvider",
        firstUri
      );
      const names = symbolNames(symbols);
      return names.some((n) => n.includes("start")) && names.some((n) => n.includes("greet"))
        ? names
        : null;
    },
    { label: "document symbols start & greet" }
  );
  assert(true, "documentSymbol lists start and greet");

  // Diagnostics: clean fixture eventually reports no diagnostics.
  await poll(
    () => {
      const diags = vscode.languages.getDiagnostics(firstUri);
      return diags.length === 0 ? true : null;
    },
    { label: "clean fixture has no diagnostics" }
  );
  assert(true, "clean fixture reports no diagnostics");

  // Completion after inserting "[" at end of document.
  const editor = await vscode.window.showTextDocument(doc);
  const endPos = doc.positionAt(doc.getText().length);
  await editor.edit((eb) => {
    eb.insert(endPos, "[");
  });
  const complPos = doc.positionAt(doc.getText().length);
  await poll(
    async () => {
      const list = await vscode.commands.executeCommand(
        "vscode.executeCompletionItemProvider",
        firstUri,
        complPos,
        "["
      );
      const labels = completionLabels(list);
      return labels.includes("jump") && labels.includes("greet") ? labels : null;
    },
    { label: "completion includes jump & greet" }
  );
  assert(true, "completion includes jump and greet");

  // Remove the inserted "[" to restore the document.
  await editor.edit((eb) => {
    const text = doc.getText();
    const lastPos = doc.positionAt(text.length);
    const prevPos = doc.positionAt(text.length - 1);
    eb.delete(new vscode.Range(prevPos, lastPos));
  });

  // Break the jump target and expect a diagnostic to appear.
  const brokenApplied = await editor.edit((eb) => {
    const text = doc.getText();
    const at = text.indexOf("target=*top");
    if (at === -1) return;
    const start = doc.positionAt(at);
    const end = doc.positionAt(at + "target=*top".length);
    eb.replace(new vscode.Range(start, end), "target=*nope");
  });
  assert(brokenApplied, "applied edit breaking the jump target");

  await poll(
    () => {
      const diags = vscode.languages.getDiagnostics(firstUri);
      return diags.length > 0 ? diags : null;
    },
    { label: "broken jump target produces a diagnostic" }
  );
  assert(true, "broken jump target produces a diagnostic");

  console.log("\nAll integration assertions passed.");
}

module.exports = { run };
