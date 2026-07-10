# TyranoScript for VS Code

Language support for [TyranoScript](https://tyrano.jp/) (`.ks`) visual-novel
scenario files, backed by the `tyrano-lsp` language server from this
repository's Rust workspace.

## Features

For TyranoScript projects with multi-file scenario semantics (labels, `[jump]`
/ `[call]` targets across files, macros, and character/asset references) this
extension provides, via the Language Server Protocol:

- Diagnostics (syntax errors, unresolved jump/call targets, unknown tags,
  missing required parameters, missing assets)
- Hover information for tags, parameters and labels
- Completion for tag names, parameter names and values
- Go to definition (labels, macros, jump/call storage targets)
- Find references
- Document symbols (labels, macros, tags) for outline / breadcrumb navigation

Syntax highlighting is provided independently via a TextMate grammar, so
`.ks` files are readable even before the language server has started
(comments, labels, character lines, inline `[tag]` / `@tag` syntax, and
embedded `[iscript]`/`[html]` blocks).

## Building the language server

The extension does not bundle the `tyrano-lsp` binary. Build it from the
workspace root:

```sh
cargo build --release -p tyrano-lsp
```

This produces `target/release/tyrano-lsp` (or `target/debug/tyrano-lsp` for
a debug build). The extension looks for the binary automatically — see
below.

## Configuring the server path

On activation the extension resolves the `tyrano-lsp` executable in this
order:

1. The `tyranoscript.server.path` setting, if non-empty.
2. `tyrano-lsp` on your `PATH`.
3. `<extension>/server/tyrano-lsp` (a bundled binary, if present).
4. `<workspace>/target/release/tyrano-lsp`.
5. `<workspace>/target/debug/tyrano-lsp`.

If no executable can be found, the extension shows an error notification
asking you to set `tyranoscript.server.path` to an absolute path, e.g. in
`.vscode/settings.json`:

```json
{
  "tyranoscript.server.path": "/absolute/path/to/tyrano-lsp"
}
```

Other settings:

- `tyranoscript.trace.server` (`off` | `messages` | `verbose`): traces
  LSP communication in the "TyranoScript" output channel.

Use the **TyranoScript: Restart Language Server** command (from the Command
Palette) after rebuilding the server or changing `tyranoscript.server.path`.

## Building and packaging the extension

From `editors/code`:

```sh
npm install
npm run compile   # bundles src/extension.ts -> dist/extension.js
npm run package   # produces tyranoscript-<version>.vsix via @vscode/vsce
```

Other scripts: `npm run typecheck` (type-checks without emitting) and
`npm run watch` (esbuild in watch mode).

## Development workflow

1. `npm install` in `editors/code`.
2. Build `tyrano-lsp` (see above), or set `tyranoscript.server.path`.
3. Open `editors/code` in VS Code and press `F5` to launch an Extension
   Development Host. This runs the `npm: compile` task first, then opens a
   new window with the extension loaded.
4. Open a folder containing `.ks` files (e.g. `editors/code/testdata`) in the
   development host window to try it out.
5. Use **Developer: Reload Window** in the development host to pick up
   further TypeScript changes after re-running `npm run compile` (or leave
   `npm run watch` running).

## Test data

`editors/code/testdata` contains a minimal two-file TyranoScript project
(`data/scenario/first.ks`, `data/scenario/scene2.ks`) exercising labels, a
macro definition and call, an `[iscript]` block, and a cross-file `[jump]`,
useful for manually exercising the grammar and the language server.

## Testing

Two end-to-end test layers live under `editors/code/test`, both driving the
`testdata` fixture project against the real `tyrano-lsp` binary. Build the
server first (`cargo build --release -p tyrano-lsp`), then from
`editors/code`:

### Layer 1 — protocol E2E (`npm run test:e2e`)

`test/e2e-protocol.mjs` is a dependency-free Node script that spawns
`../../../target/release/tyrano-lsp` and speaks raw LSP JSON-RPC (hand-rolled
`Content-Length` framing) with `testdata` as the workspace root. It asserts,
against the actual fixture files, that `initialize` advertises the hover
provider, and that hover, go-to-definition, completion (tag names + the
`greet` macro), find-references (spanning both files), document symbols and
`publishDiagnostics` all return the expected results, then verifies a clean
`shutdown`/`exit` with process exit code 0. All LSP positions are derived
programmatically from the fixture text (UTF-16 code units), not hardcoded.

```sh
npm run test:e2e
```

### Layer 2 — VS Code integration (`npm run test:integration`)

`test/runTest.mjs` uses `@vscode/test-electron` to download a throwaway VS
Code, copy the built server binary to `editors/code/server/tyrano-lsp` (so the
extension's `<extension>/server/tyrano-lsp` lookup finds it — `server/` is
git-ignored), and launch the extension host against `testdata`. The suite
(`test/suite.js`, loaded by VS Code as CommonJS) activates the extension and
exercises the hover, definition, completion, document-symbol and diagnostics
providers via `vscode.commands.executeCommand`, including breaking a `[jump]`
target to confirm a diagnostic appears.

This test needs a display; run it headlessly with `xvfb-run`:

```sh
xvfb-run -a node test/runTest.mjs   # or: xvfb-run -a npm run test:integration
```

It downloads VS Code from `update.code.visualstudio.com` on first run, so it
requires network access to that host (some sandboxed/CI egress policies block
it).
