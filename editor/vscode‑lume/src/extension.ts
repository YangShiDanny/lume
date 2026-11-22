import * as vscode from "vscode";
import * as path from "path";

const keywords = ["fn", "ret", "if", "else", "while", "and", "or", "not", "is", "true", "false", "end"];
const types = ["int", "i16", "i32", "i64", "float", "string", "bool", "pointer"];
const builtins = ["print", "len"];

function getBootstrap(context: vscode.ExtensionContext): string {
  if (process.env.LUME_BOOTSTRAP) return process.env.LUME_BOOTSTRAP;
  const workspace = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  if (workspace) return path.join(workspace, "target", "debug", "lume-bootstrap");
  return path.resolve(context.extensionPath, "..", "..", "..", "target", "debug", "lume-bootstrap");
}

function stripComment(line: string): string {
  let quote = false;
  for (let i = 0; i < line.length; i++) {
    if (line[i] === "\"" && line[i - 1] !== "\\") quote = !quote;
    if (line[i] === "#" && !quote) return line.slice(0, i);
  }
  return line;
}

function diagnose(document: vscode.TextDocument): vscode.Diagnostic[] {
  const diagnostics: vscode.Diagnostic[] = [];
  let blockDepth = 0;
  let functions = 0;
  for (let i = 0; i < document.lineCount; i++) {
    const raw = document.lineAt(i).text;
    const line = stripComment(raw).trim();
    if (!line) continue;
    if (line.startsWith("fn ")) {
      functions++;
      if (!/^fn\s+[A-Za-z_][A-Za-z0-9_]*\s*\([^)]*\)(?:\s*->\s*[A-Za-z0-9_*]+)?\s*$/.test(line)) {
        diagnostics.push(new vscode.Diagnostic(
          new vscode.Range(i, 0, i, raw.length),
          "Invalid function declaration",
          vscode.DiagnosticSeverity.Error
        ));
      }
    }
    if (/^(?:if|while)\b/.test(line)) blockDepth++;
    if (/^else(?:\s+if\b|$)/.test(line)) blockDepth++;
    if (/^res:\w+\s*=/.test(line)) {
      if (/=\s*(?:if|while)\b/.test(line)) blockDepth++;
    }
    if (line === "end") blockDepth--;
  }
  if (blockDepth > 0) {
    const i = document.lineCount - 1;
    diagnostics.push(new vscode.Diagnostic(
      new vscode.Range(i, 0, i, 0),
      "Unclosed block: expected end",
      vscode.DiagnosticSeverity.Error
    ));
  }
  if (functions === 0) {
    diagnostics.push(new vscode.Diagnostic(
      new vscode.Range(0, 0, 0, document.lineCount > 0 ? document.lineAt(0).text.length : 0),
      "Lume source should contain at least one fn declaration",
      vscode.DiagnosticSeverity.Warning
    ));
  }
  return diagnostics;
}

export function activate(context: vscode.ExtensionContext) {
  const diagnostics = vscode.languages.createDiagnosticCollection("lume");
  context.subscriptions.push(diagnostics);
  const validate = (document: vscode.TextDocument) => {
    if (document.languageId === "lume") diagnostics.set(document.uri, diagnose(document));
  };
  context.subscriptions.push(vscode.workspace.onDidOpenTextDocument(validate));
  context.subscriptions.push(vscode.workspace.onDidSaveTextDocument(validate));
  context.subscriptions.push(vscode.workspace.onDidChangeTextDocument(event => validate(event.document)));
  vscode.workspace.textDocuments.forEach(validate);

  context.subscriptions.push(vscode.languages.registerCompletionItemProvider(
    { language: "lume" },
    {
      provideCompletionItems(document, position) {
        const line = document.lineAt(position).text.slice(0, position.character);
        const prefix = line.match(/[A-Za-z_][A-Za-z0-9_]*$/)?.[0] ?? "";
        const items: vscode.CompletionItem[] = [];
        for (const word of [...keywords, ...types, ...builtins]) {
          const item = new vscode.CompletionItem(word, completionKind(word));
          item.range = prefix
            ? new vscode.Range(position.with(undefined, position.character - prefix.length), position)
            : undefined;
          items.push(item);
        }
        for (let i = 0; i < document.lineCount; i++) {
          const match = stripComment(document.lineAt(i).text).trim().match(
            /^fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)\)(?:\s*->\s*([A-Za-z0-9_*]+))?/
          );
          if (!match) continue;
          const item = new vscode.CompletionItem(match[1], vscode.CompletionItemKind.Function);
          item.detail = match[3] ? `${match[1]}(...)->${match[3]}` : `${match[1]}(...)`;
          items.push(item);
        }
        return items;
      }
    }
  ));

  context.subscriptions.push(vscode.languages.registerHoverProvider(
    { language: "lume" },
    {
      provideHover(document, position) {
        const range = document.getWordRangeAtPosition(position);
        if (!range) return;
        const word = document.getText(range);
        if ([...keywords, ...types, ...builtins].includes(word)) {
          return new vscode.Hover(`**${word}** · ${wordDescription(word)}`);
        }
        for (let i = 0; i < document.lineCount; i++) {
          const match = stripComment(document.lineAt(i).text).trim().match(
            /^fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)\)(?:\s*->\s*([A-Za-z0-9_*]+))?/
          );
          if (match?.[1] === word) {
            return new vscode.Hover(
              `\`\`\`lume\nfn ${match[1]}(${match[2]})${match[3] ? ` -> ${match[3]}` : ""}\n\`\`\``
            );
          }
        }
      }
    }
  ));

  context.subscriptions.push(vscode.languages.registerDocumentSymbolProvider(
    { language: "lume" },
    {
      provideDocumentSymbols(document) {
        const symbols: vscode.DocumentSymbol[] = [];
        for (let i = 0; i < document.lineCount; i++) {
          const match = stripComment(document.lineAt(i).text).trim().match(
            /^fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)\)(?:\s*->\s*([A-Za-z0-9_*]+))?/
          );
          if (!match) continue;
          const line = document.lineAt(i);
          const start = line.text.indexOf(match[1]);
          symbols.push(new vscode.DocumentSymbol(
            match[1],
            match[3] ? `-> ${match[3]}` : "function",
            vscode.SymbolKind.Function,
            line.range,
            new vscode.Range(i, start, i, start + match[1].length)
          ));
        }
        return symbols;
      }
    }
  ));

  context.subscriptions.push(
    vscode.commands.registerCommand("lume.runFile", () => runOrCheck(context, "run")),
    vscode.commands.registerCommand("lume.checkFile", () => runOrCheck(context, "check"))
  );
}

function completionKind(word: string): vscode.CompletionItemKind {
  if (types.includes(word)) return vscode.CompletionItemKind.TypeParameter;
  if (builtins.includes(word)) return vscode.CompletionItemKind.Function;
  return vscode.CompletionItemKind.Keyword;
}

function wordDescription(word: string): string {
  const descriptions: Record<string, string> = {
    fn: "declare a function",
    ret: "return from the current function",
    if: "conditional block or block expression",
    else: "alternate conditional branch",
    while: "loop block or block expression",
    and: "short-circuit boolean and",
    or: "short-circuit boolean or",
    not: "boolean negation",
    is: "identity comparison",
    true: "boolean true",
    false: "boolean false",
    end: "close a block",
    print: "print one value",
    len: "length of a string or array",
  };
  return descriptions[word] ?? "Lume type";
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
