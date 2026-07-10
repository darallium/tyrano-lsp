import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;
let outputChannel: vscode.OutputChannel | undefined;

/**
 * Resolves the path to the `tyrano-lsp` server executable.
 *
 * Search order:
 *   1. The `tyranoscript.server.path` setting, if set.
 *   2. `tyrano-lsp` on PATH.
 *   3. `<extension>/server/tyrano-lsp`.
 *   4. `<workspace>/target/release/tyrano-lsp`.
 *   5. `<workspace>/target/debug/tyrano-lsp`.
 *
 * Returns `undefined` when no candidate can be found.
 */
function resolveServerPath(context: vscode.ExtensionContext): string | undefined {
  const exe = os.platform() === "win32" ? "tyrano-lsp.exe" : "tyrano-lsp";
  const config = vscode.workspace.getConfiguration("tyranoscript");

  const configured = config.get<string>("server.path", "").trim();
  if (configured.length > 0) {
    return configured;
  }

  const fileCandidates: string[] = [
    path.join(context.extensionPath, "server", exe),
  ];

  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    fileCandidates.push(path.join(folder.uri.fsPath, "target", "release", exe));
    fileCandidates.push(path.join(folder.uri.fsPath, "target", "debug", exe));
  }

  for (const candidate of fileCandidates) {
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }

  // Fall back to a bare command name resolved via PATH. We cannot check
  // existence for this case, so it is only used when nothing else matched.
  return "tyrano-lsp";
}

function log(message: string): void {
  if (!outputChannel) {
    outputChannel = vscode.window.createOutputChannel("TyranoScript");
  }
  outputChannel.appendLine(message);
}

async function startClient(context: vscode.ExtensionContext): Promise<void> {
  const serverPath = resolveServerPath(context);

  if (!serverPath) {
    vscode.window.showErrorMessage(
      "TyranoScript: could not find the tyrano-lsp language server. " +
        "Set 'tyranoscript.server.path' in your settings to the absolute path of the tyrano-lsp executable."
    );
    return;
  }

  // When the resolved path looks like an absolute/file path (not a bare
  // command name expected to be on PATH) but does not actually exist,
  // warn the user clearly instead of silently failing to spawn.
  const looksLikeFilePath = path.isAbsolute(serverPath) || serverPath.includes(path.sep);
  if (looksLikeFilePath && !fs.existsSync(serverPath)) {
    vscode.window.showErrorMessage(
      `TyranoScript: the configured tyrano-lsp server executable was not found at "${serverPath}". ` +
        "Build it with 'cargo build --release -p tyrano-lsp' or set 'tyranoscript.server.path'."
    );
    return;
  }

  log(`Starting tyrano-lsp from: ${serverPath}`);

  const serverOptions: ServerOptions = {
    run: { command: serverPath, transport: TransportKind.stdio },
    debug: { command: serverPath, transport: TransportKind.stdio },
  };

  const config = vscode.workspace.getConfiguration("tyranoscript");

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "tyranoscript" }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.ks"),
    },
    outputChannel,
    traceOutputChannel: outputChannel,
    initializationOptions: {
      trace: config.get<string>("trace.server", "off"),
    },
  };

  client = new LanguageClient(
    "tyranoscript",
    "TyranoScript Language Server",
    serverOptions,
    clientOptions
  );

  try {
    await client.start();
  } catch (err) {
    vscode.window.showErrorMessage(
      `TyranoScript: failed to start the tyrano-lsp language server: ${String(err)}`
    );
  }
}

async function stopClient(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}

async function restartServer(context: vscode.ExtensionContext): Promise<void> {
  log("Restarting tyrano-lsp...");
  await stopClient();
  await startClient(context);
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  context.subscriptions.push(
    vscode.commands.registerCommand("tyranoscript.restartServer", () => restartServer(context))
  );

  await startClient(context);
}

export async function deactivate(): Promise<void> {
  await stopClient();
}
