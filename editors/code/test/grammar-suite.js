// Grammar suite: runs inside the VS Code extension host and verifies the
// TextMate grammar (syntaxes/tyranoscript.tmLanguage.json) against the real
// tokenizer, including embedded HTML/JS grammars, via the internal
// `_workbench.captureSyntaxTokens` command. No language server is required:
// grammars are declarative contributions, active without extension code.

const path = require("path");
const vscode = require("vscode");

function assert(cond, message) {
  if (!cond) {
    throw new Error(`Assertion failed: ${message}`);
  }
  console.log(`  ok - ${message}`);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function poll(fn, { timeout = 60000, interval = 300, label = "condition" } = {}) {
  const start = Date.now();
  for (;;) {
    const v = await fn();
    if (v) return v;
    if (Date.now() - start > timeout) {
      throw new Error(`timeout waiting for ${label}`);
    }
    await sleep(interval);
  }
}

// `_workbench.captureSyntaxTokens` returns a flat token list for the whole
// document: [{ c: content, t: scopes-joined-by-space, r: theme rules }].
// Group the tokens back into lines using the document text.
function groupTokensByLine(docText, tokens) {
  const lines = docText.split(/\r?\n/).map((text) => ({ text, tokens: [] }));
  let line = 0;
  let col = 0;
  for (const tok of tokens) {
    // Advance past lines already fully covered (and empty lines).
    while (line < lines.length && col >= lines[line].text.length) {
      line++;
      col = 0;
    }
    if (line >= lines.length) break;
    lines[line].tokens.push(tok);
    col += tok.c.length;
  }
  return lines;
}

function lineByPrefix(lines, prefix) {
  const entry = lines.find((l) => l.text.trimStart().startsWith(prefix));
  if (!entry) throw new Error(`no line starting with ${JSON.stringify(prefix)}`);
  return entry;
}

// Assert that on `line` the token whose content trims to `content` carries
// `scope` (substring match against the space-joined scope list).
function assertTokenScope(line, content, scope, message) {
  const tok = line.tokens.find((t) => t.c.trim() === content);
  assert(tok, `${message}: token ${JSON.stringify(content)} exists on ${JSON.stringify(line.text.trim())}`);
  assert(
    tok.t.includes(scope),
    `${message}: ${JSON.stringify(content)} has scope ${scope} (got: ${tok.t})`
  );
}

function assertLineScope(line, scope, message) {
  const all = line.tokens.map((t) => t.t).join(" ");
  assert(all.includes(scope), `${message} (scopes on line: ${all || "<none>"})`);
}

function assertLineNoScope(line, scope, message) {
  const all = line.tokens.map((t) => t.t).join(" ");
  assert(!all.includes(scope), `${message} (scopes on line: ${all || "<none>"})`);
}

async function run() {
  const folders = vscode.workspace.workspaceFolders;
  if (!folders || folders.length === 0) throw new Error("no workspace folder open");
  const root = folders[0].uri.fsPath;
  const uri = vscode.Uri.file(path.join(root, "data", "scenario", "grammar.ks"));

  const doc = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(doc);
  assert(doc.languageId === "tyranoscript", `grammar.ks opens as tyranoscript (got ${doc.languageId})`);

  // Tokenization (and the embedded HTML/JS grammars) load asynchronously;
  // poll until the tag name token carries the grammar's scope.
  const tokens = await poll(
    async () => {
      const toks = await vscode.commands.executeCommand("_workbench.captureSyntaxTokens", uri);
      if (!Array.isArray(toks) || toks.length === 0) return null;
      const ready =
        toks.some((t) => t.t.includes("entity.name.tag.tyranoscript")) &&
        toks.some((t) => t.t.includes("meta.embedded.block.html")) &&
        toks.some((t) => t.t.includes("meta.embedded.block.javascript"));
      return ready ? toks : null;
    },
    { label: "syntax tokens with tyranoscript + embedded scopes" }
  );
  const lines = groupTokensByLine(doc.getText(), tokens);

  // --- quotes ------------------------------------------------------------
  const quotes = lineByPrefix(lines, '[ptext text="a b"');
  assertTokenScope(quotes, '"a b"', "string.quoted.double.tyranoscript", "double-quoted value");
  assertTokenScope(quotes, "'c d'", "string.quoted.single.tyranoscript", "single-quoted value");
  assertTokenScope(quotes, "`e f`", "string.quoted.other.backtick.tyranoscript", "backtick-quoted value");

  const hidden = lineByPrefix(lines, '[ptext text="[[');
  assertTokenScope(hidden, '"[[あ]]"', "string.quoted.double.tyranoscript", "brackets inside double quotes");
  assertTokenScope(hidden, "`x]y`", "string.quoted.other.backtick.tyranoscript", "bracket inside backticks");
  assertTokenScope(hidden, "]", "punctuation.definition.tag.end.tyranoscript", "tag still closes after quoted ]");

  const eqval = lineByPrefix(lines, '[eval exp="f.a=1"');
  assertTokenScope(eqval, '"f.a=1"', "string.quoted.double.tyranoscript", "= inside quoted value stays in the string");
  assertTokenScope(eqval, "time", "variable.parameter.tyranoscript", "parameter after quoted value");

  const afterUnterminated = lineByPrefix(lines, "plain line after unterminated quote");
  assertLineNoScope(
    afterUnterminated,
    "string.quoted",
    "unterminated quote does not leak onto the next line"
  );
  assertTokenScope(afterUnterminated, "l", "entity.name.tag.tyranoscript", "inline tag works after unterminated quote");

  // --- html block ---------------------------------------------------------
  const htmlOpen = lineByPrefix(lines, "[html top=");
  assertTokenScope(htmlOpen, "html", "entity.name.tag.tyranoscript", "[html] with params is a tag");
  assertTokenScope(htmlOpen, "top", "variable.parameter.tyranoscript", "html param name");
  assertTokenScope(htmlOpen, '"0"', "string.quoted.double.tyranoscript", "html param value");

  const divLine = lineByPrefix(lines, "<div style=");
  assertLineScope(divLine, "meta.embedded.block.html", "html block content is embedded HTML");
  assertLineScope(divLine, ".html", "html content is highlighted by text.html.basic");

  const htmlClose = lineByPrefix(lines, "[endhtml]");
  assertTokenScope(htmlClose, "endhtml", "entity.name.tag.tyranoscript", "[endhtml] closes the block");

  const afterHtml = lineByPrefix(lines, "text after endhtml");
  assertLineNoScope(afterHtml, "meta.embedded.block.html", "scenario text resumes after [endhtml]");
  assertTokenScope(afterHtml, "l", "entity.name.tag.tyranoscript", "inline tag works after [endhtml]");

  const notABlock = lineByPrefix(lines, "not a block line");
  assertLineNoScope(notABlock, "meta.embedded.block.html", "[html2] does not open an html block");

  // --- iscript block -------------------------------------------------------
  const jsOpen = lineByPrefix(lines, "[iscript stop=true]");
  assertTokenScope(jsOpen, "iscript", "entity.name.tag.tyranoscript", "[iscript] with params is a tag");

  const jsLine = lineByPrefix(lines, "var a = 1;");
  assertLineScope(jsLine, "meta.embedded.block.javascript", "iscript content is embedded JS");
  assertTokenScope(jsLine, "var", "storage.type", "iscript content is highlighted by source.js");

  const jsClose = lineByPrefix(lines, "[endscript foo=1]");
  assertTokenScope(jsClose, "endscript", "entity.name.tag.tyranoscript", "[endscript foo=1] closes the block");
  assertTokenScope(jsClose, "foo", "variable.parameter.tyranoscript", "endscript param name");

  const afterJs = lineByPrefix(lines, "text after endscript");
  assertLineNoScope(afterJs, "meta.embedded.block.javascript", "scenario text resumes after [endscript]");

  const atJs = lineByPrefix(lines, "var b = 2;");
  assertLineScope(atJs, "meta.embedded.block.javascript", "@iscript opens a JS block");
  const theEnd = lineByPrefix(lines, "the end");
  assertLineNoScope(theEnd, "meta.embedded.block.javascript", "@endscript closes the JS block");

  console.log("\nAll grammar assertions passed.");
}

module.exports = { run };
