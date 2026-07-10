"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.activate = activate;
exports.deactivate = deactivate;
const vscode = require("vscode");
const node_1 = require("vscode-languageclient/node");
let client;
function resolveServerOptions() {
    const config = vscode.workspace.getConfiguration('tyranoScript');
    const explicitPath = config.get('server.path', '').trim();
    const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
    const cwd = workspaceFolder?.uri.fsPath;
    if (explicitPath) {
        return {
            command: explicitPath,
            transport: node_1.TransportKind.stdio,
            options: cwd ? { cwd } : undefined,
        };
    }
    const useCargoRun = config.get('server.useCargoRun', true);
    if (useCargoRun && cwd) {
        return {
            command: 'C:/workspace/2025/rust/tyrano-parser/target/release/tyrano-server.exe',
            transport: node_1.TransportKind.stdio,
            options: { cwd },
        };
    }
    return { command: 'tyrano-server', transport: node_1.TransportKind.stdio };
}
function activate(context) {
    const serverOptions = resolveServerOptions();
    const clientOptions = {
        documentSelector: [{ scheme: 'file', language: 'tyranoscript' }],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher('**/*.ks'),
        },
        traceOutputChannel: vscode.window.createOutputChannel('TyranoScript Language Server'),
    };
    client = new node_1.LanguageClient('tyranoScriptLanguageServer', 'TyranoScript Language Server', serverOptions, clientOptions);
    client.start().catch((err) => {
        const message = err instanceof Error ? err.message : String(err);
        vscode.window.showErrorMessage(`Failed to start tyrano-server: ${message}. ` +
            'Set "tyranoScript.server.path" to a built tyrano-server executable, ' +
            'or run `cargo build -p tyrano-server` in the workspace root.');
    });
    context.subscriptions.push({
        dispose: () => {
            void client?.stop();
        },
    });
}
function deactivate() {
    return client?.stop();
}
//# sourceMappingURL=extension.js.map