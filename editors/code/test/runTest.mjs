#!/usr/bin/env node
// Layer 2 driver: downloads VS Code (via @vscode/test-electron), launches it
// headlessly with this extension loaded against the testdata workspace, and
// runs test/suite.js inside the VS Code extension host.
//
// Headless/CI usage:  xvfb-run -a node test/runTest.mjs   (npm run test:integration)

import { fileURLToPath } from "node:url";
import { copyFileSync, mkdirSync, chmodSync } from "node:fs";
import path from "node:path";
import { runTests } from "@vscode/test-electron";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

async function main() {
  const extensionDevelopmentPath = path.resolve(__dirname, "..");
  const extensionTestsPath = path.resolve(__dirname, "./suite.js");
  const workspacePath = path.resolve(__dirname, "../testdata");

  // The extension resolves the server from <extension>/server/tyrano-lsp
  // (among other places). The test workspace (testdata) has no target/ dir,
  // so the most robust option is to copy the prebuilt binary there before
  // launching. editors/code/server/ is git-ignored.
  const exe = process.platform === "win32" ? "tyrano-lsp.exe" : "tyrano-lsp";
  const builtBinary = path.resolve(__dirname, "../../../target/release", exe);
  const serverDir = path.resolve(__dirname, "../server");
  const serverBinary = path.join(serverDir, exe);
  mkdirSync(serverDir, { recursive: true });
  copyFileSync(builtBinary, serverBinary);
  chmodSync(serverBinary, 0o755);
  console.log(`Copied server binary -> ${serverBinary}`);

  await runTests({
    extensionDevelopmentPath,
    extensionTestsPath,
    launchArgs: [
      workspacePath,
      "--disable-extensions",
      "--disable-gpu",
      "--no-sandbox",
    ],
  });
}

main().catch((err) => {
  console.error("Integration test run failed:");
  console.error(err && err.stack ? err.stack : String(err));
  process.exit(1);
});
