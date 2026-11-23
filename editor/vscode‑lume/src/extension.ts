import * as vscode from "vscode";
import * as path from "path";
import { LanguageClient, TransportKind } from "vscode-languageclient/node";

type Builtin = { name: string; signature: string; doc: string };
const builtins: Builtin[] = [
  { name: "print", signature: "print(value) -> unit", doc: "Print one value to stdout." },
  { name: "len", signature: "len(value) -> int", doc: "Length of a string or array." },
  ...[
    "sin", "cos", "tan", "asin", "acos", "atan", "sinh", "cosh", "tanh", "asinh", "acosh", "atanh",
    "exp", "exp2", "expm1", "ln", "log", "log2", "log10", "log1p", "sqrt", "cbrt", "floor", "ceil",
    "round", "trunc", "abs",
  ].map((name) => ({ name, signature: `${name}(x:float) -> float`, doc: `Math unary function \`${name}\`.` })),
  ...["pow", "atan2", "hypot", "fmod", "remainder", "copysign"].map((name) => ({
    name,
    signature: `${name}(x:float, y:float) -> float`,
    doc: `Math binary function \`${name}\`.`,
  })),
  ...["trace", "debug", "info", "warn", "error"].map((name) => ({
    name,
    signature: `${name}(message)`,
    doc: `Log a ${name} message to stderr.`,
  })),
  ...["log_trace", "log_debug", "log_info", "log_warn", "log_error"].map((name) => ({
    name,
    signature: `${name}(message)`,
    doc: `Log with explicit ${name.replace("log_", "")} level.`,
  })),
  { name: "log_at", signature: "log_at(level:string, message)", doc: "Log at a runtime-provided level." },
];

const builtinMap = new Map(builtins.map((item) => [item.name, item]));

function getBootstrap(context: vscode.ExtensionContext): string {
  const configured = vscode.workspace.getConfiguration("lume").get<string>("bootstrapPath");
  if (configured) return configured;
  if (process.env.LUME_BOOTSTRAP) return process.env.LUME_BOOTSTRAP;
  const workspace = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  if (workspace) {
    const name = process.platform === "win32" ? "lume-bootstrap.exe" : "lume-bootstrap";
    return path.join(workspace, "target", "release", name);
  }
  return path.resolve(context.extensionPath, "..", "..", "..", "target", "release", "lume-bootstrap");
}

function stripComment(line: string): string {
  let quote = false;
  for (let i = 0; i < line.length; i++) {
    if (line[i] === "\"" && line[i - 1] !== "\\") quote = !quote;
    if (line[i] === "#" && !quote) return line.slice(0, i);
  }
  return line;
}

type FunctionInfo = { name: string; params: string; ret?: string; line: number };
type VariableInfo = { name: string; type?: string; line: number };

function collectSymbols(document: vscode.TextDocument): { functions: FunctionInfo[]; variables: VariableInfo[] } {
  const functions: FunctionInfo[] = [];
  const variables: VariableInfo[] = [];
  const variableNames = new Set<string>();

  const functionPattern = /^fn\s+([A-Za-z_]\w*)\s*\(([^)]*)\)(?:\s*->\s*([A-Za-z0-9_*]+))?/;
  const variablePattern = /^\s*([A-Za-z_]\w*)\s*(?::\s*([A-Za-z_]\w*))?\s*(?::=|=)\s*(.+)$/;
  const paramPattern = /([A-Za-z_]\w*)\s*:\s*[A-Za-z_]\w*/g;

  for (let i = 0; i < document.lineCount; i++) {
    const line = stripComment(document.lineAt(i).text).trim();
    const fnMatch = line.match(functionPattern);
    if (fnMatch) {
      functions.push({ name: fnMatch[1], params: fnMatch[2], ret: fnMatch[3], line: i });
      for (const match of fnMatch[2].matchAll(paramPattern)) {
        const name = match[1];
        if (!variableNames.has(name)) {
          variableNames.add(name);
          variables.push({ name, line: i });
        }
      }
      continue;
    }
    const varMatch = line.match(variablePattern);
    if (varMatch) {
      const name = varMatch[1];
      if (!variableNames.has(name)) {
        variableNames.add(name);
        variables.push({ name, type: varMatch[2], line: i });
      }
    }
  }
  return { functions, variables };
}

export function activate(context: vscode.ExtensionContext) {
  const serverOptions = {
    command: getBootstrap(context),
    args: ["--lsp"],
    transport: TransportKind.stdio,
  };
  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "lume" }],
  };
  const client = new LanguageClient("lume", "Lume Language Server", serverOptions, clientOptions);
  context.subscriptions.push(client);
  void client.start().catch((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    void vscode.window.showErrorMessage(`Lume language server failed to start: ${message}`);
  });

  context.subscriptions.push(vscode.languages.registerSignatureHelpProvider(
    { language: "lume" },
    {
      provideSignatureHelp(document, position) {
        const before = document.getText(new vscode.Range(0, 0, position.line, position.character));
        const open = before.lastIndexOf("(");
        if (open < 0) return undefined;
        const nameMatch = before.slice(0, open).match(/([A-Za-z_]\w*)\s*$/);
        if (!nameMatch) return undefined;
        const name = nameMatch[1];
        const argsText = before.slice(open + 1);
        const activeParameter = (argsText.match(/,/g) ?? []).length;

        let label: string | undefined;
        let doc: string | undefined;
        const builtin = builtinMap.get(name);
        if (builtin) {
          label = builtin.signature;
          doc = builtin.doc;
        } else {
          const symbol = collectSymbols(document).functions.find((fn) => fn.name === name);
          if (symbol) {
            label = `fn ${name}(${symbol.params})${symbol.ret ? ` -> ${symbol.ret}` : ""}`;
            doc = `Defined on line ${symbol.line + 1}`;
          }
        }
        if (!label) return undefined;

        const signature = new vscode.SignatureInformation(label, doc);
        signature.parameters = label
          .slice(label.indexOf("(") + 1, label.lastIndexOf(")"))
          .split(",")
          .map((part) => new vscode.ParameterInformation(part.trim()))
          .filter((param) => param.label.length > 0);
        const help = new vscode.SignatureHelp();
        help.signatures = [signature];
        help.activeSignature = 0;
        help.activeParameter = Math.min(activeParameter, Math.max(signature.parameters.length - 1, 0));
        return help;
      }
    },
    "(", ","
  ));

  context.subscriptions.push(vscode.languages.registerDocumentSymbolProvider(
    { language: "lume" },
    {
      provideDocumentSymbols(document) {
        const symbols: vscode.DocumentSymbol[] = [];
        for (const fn of collectSymbols(document).functions) {
          const line = document.lineAt(fn.line);
          const start = line.text.indexOf(fn.name);
          symbols.push(new vscode.DocumentSymbol(
            fn.name,
            fn.ret ? `-> ${fn.ret}` : "function",
            vscode.SymbolKind.Function,
            line.range,
            new vscode.Range(fn.line, start, fn.line, start + fn.name.length)
          ));
        }
        return symbols;
      }
    }
  ));

  context.subscriptions.push(
    vscode.commands.registerCommand("lume.runFile", () => runOrCheck(context, "run")),
    vscode.commands.registerCommand("lume.checkFile", () => runOrCheck(context, "check")),
    vscode.commands.registerCommand("lume.openRepl", () => {
      const command = vscode.workspace.getConfiguration("lume").get<string>("replCommand") || "lume";
      const terminal = vscode.window.createTerminal("Lume REPL");
      terminal.show();
      terminal.sendText(command);
    })
  );
}

function runOrCheck(context: vscode.ExtensionContext, command: "run" | "check") {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.document.languageId !== "lume") {
    vscode.window.showWarningMessage("Open a Lume (.lume) file first.");
    return;
  }
  void editor.document.save();
  const terminal = vscode.window.createTerminal("Lume");
  terminal.show();
  terminal.sendText(`"${getBootstrap(context)}" ${command} "${editor.document.uri.fsPath}"`);
}

export function deactivate() {}
