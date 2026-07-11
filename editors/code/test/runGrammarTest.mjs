#!/usr/bin/env node
// Grammar test driver: launches VS Code (via @vscode/test-electron) with this
// extension loaded against the testdata workspace and runs
// test/grammar-suite.js inside the extension host. Unlike runTest.mjs this
// needs no tyrano-lsp binary: the TextMate grammar is a declarative
// contribution and is exercised without activating the extension code.
//
// Headless/CI usage:  xvfb-run -a node test/runGrammarTest.mjs

import { fileURLToPath } from "node:url";
import path from "node:path";
import { runTests } from "@vscode/test-electron";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

async function main() {
  const extensionDevelopmentPath = path.resolve(__dirname, "..");
  const extensionTestsPath = path.resolve(__dirname, "./grammar-suite.js");
  const workspacePath = path.resolve(__dirname, "../testdata");

  await runTests({
    extensionDevelopmentPath,
    extensionTestsPath,
    launchArgs: [
      workspacePath,
      "--disable-extensions",
      "--disable-gpu",
      "--no-sandbox",
      "--disable-workspace-trust",
    ],
  });
}

main().catch((err) => {
  console.error("Grammar test run failed:");
  console.error(err && err.stack ? err.stack : String(err));
  process.exit(1);
});
