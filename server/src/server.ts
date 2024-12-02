import {
  createConnection,
  TextDocuments,
  Diagnostic,
  DiagnosticSeverity,
  ProposedFeatures,
  InitializeParams,
  DidChangeConfigurationNotification,
  TextDocumentSyncKind,
  InitializeResult,
  Range,
  Position,
} from "vscode-languageserver/node";

import { TextDocument } from "vscode-languageserver-textdocument";

import { TyranoScriptParser } from "./parser";
import { TyranoScriptFormatter, FormatterOptions } from "./formatter";
import { TyranoScriptLinter } from "./linter";

// Create a connection for the server, using Node's IPC as a transport.
// Also include all preview / proposed LSP features.
const connection = createConnection(ProposedFeatures.all);

// Create a simple text document manager.
const documents = new TextDocuments<TextDocument>(TextDocument);

let hasConfigurationCapability = false;
let hasWorkspaceFolderCapability = false;

connection.onInitialize((params: InitializeParams) => {
  const capabilities = params.capabilities;

  // Does the client support the `workspace/configuration` request?

  // If so, we should read the configuration from the client.

  // If not, we will use the default settings.

  hasConfigurationCapability = !!(
    capabilities.workspace && !!capabilities.workspace.configuration
  );

  hasWorkspaceFolderCapability = !!(
    capabilities.workspace && !!capabilities.workspace.workspaceFolders
  );


  const result: InitializeResult = {
    capabilities: {
      textDocumentSync: TextDocumentSyncKind.Incremental,

      // Tell the client that this server supports code completion.

      completionProvider: {
        resolveProvider: true,
      },
      documentFormattingProvider: true,
    },
  };

  if (hasWorkspaceFolderCapability) {
    result.capabilities.workspace = {
      workspaceFolders: {
        supported: true,
      },
    };
  }

  return result;
});

connection.onInitialized(() => {
  if (hasConfigurationCapability) {
    // Register for all configuration changes.

    connection.client.register(
      DidChangeConfigurationNotification.type,
      undefined
    );
  }
});

// The global settings, used when the `workspace/configuration` request is not supported by the client.

// Please note that this is not the case when using this server with the client provided in this example

// but could happen with other clients.

const defaultSettings: FormatterOptions = {
  indentSize: 2,
  useTabs: false,
  tagAttributesOnNewLine: true,
  newline: "\r\n",
};
let globalSettings: FormatterOptions = defaultSettings;

// Cache the settings of all open documents

const documentSettings = new Map<string, Thenable<FormatterOptions>>();

connection.onDidChangeConfiguration((change) => {
  if (hasConfigurationCapability) {
    // Reset all cached document settings

    documentSettings.clear();
  } else {
    globalSettings = (change.settings.tyranoscriptLanguageServer || defaultSettings) as FormatterOptions;
  }

  // Revalidate all open text documents

  documents.all().forEach(validateTextDocument);
});

function getDocumentSettings(resource: string): Thenable<FormatterOptions> {
  if (!hasConfigurationCapability) {
    return Promise.resolve(globalSettings);
  }

  let result = documentSettings.get(resource);

  if (!result) {
    result = connection.workspace.getConfiguration({
      scopeUri: resource,

      section: "tyranoscriptLanguageServer",
    });

    documentSettings.set(resource, result);
  }

  return result;
}

// Only keep settings for open documents

documents.onDidClose((e) => {
  documentSettings.delete(e.document.uri);
});

// The content of a text document has changed. This event is emitted

// when the text document first opened or when its content has changed.

documents.onDidChangeContent((change) => {
  validateTextDocument(change.document);
});

const parser = new TyranoScriptParser();

const linter = new TyranoScriptLinter();

async function validateTextDocument(textDocument: TextDocument): Promise<void> {
  // In this simple example we get the settings for every validate run.

  const settings = await getDocumentSettings(textDocument.uri);

  const formatter = new TyranoScriptFormatter(settings);

  // The validator creates diagnostics for all uppercase words length 2 and more

  const text = textDocument.getText();
  const pattern = /\b[A-Z]{2,}\b/g;
  let m: RegExpExecArray | null;

  const problems = 0;

  const diagnostics: Diagnostic[] = [];

  //Lint

  const ast = parser.parse(text);
  const lint_errors = linter.lint(ast);


  for(const err of lint_errors){
    const diagnosic: Diagnostic = {
      severity: DiagnosticSeverity.Error,

      range: {
        start: Position.create(err.line, err.column),

        end: Position.create(err.line, err.column + 1),
      },

      message: err["message"],

      source: "ex",
    };

    diagnostics.push(diagnosic);

  }

  // Send the computed diagnostics to VSCode.

  connection.sendDiagnostics({ uri: textDocument.uri, diagnostics });
}

connection.onDocumentFormatting(async (params, _token) => {
  const document = documents.get(params.textDocument.uri);

  if (document) {
    const settings = await getDocumentSettings(document.uri);

    const formatter = new TyranoScriptFormatter(settings);

	//console.log(document.getText());
	console.log(parser.parse(document.getText()));
	const ast = (() => {
		try{
			return parser.parse(document.getText());
		// eslint-disable-next-line @typescript-eslint/no-unused-vars
		}catch(_){
			return null;
		};
	})();
	if(!ast){
		return ;
	}

	console.log(ast);
    const formattedText = formatter.format(ast);
	console.log(formattedText);

    const range = Range.create(
      Position.create(0, 0),
      document.positionAt(document.getText().length)
    );

    return [
      {
        range: range,
        newText: formattedText,
      },
    ];
  }

  return null;
});

// Make the text document manager listen on the connection

// for open, change and close text document events

documents.listen(connection);

// Listen on the connection

connection.listen();
