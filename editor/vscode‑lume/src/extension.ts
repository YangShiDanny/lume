import * as vscode from "vscode";
import * as path from "path";

// 与 docs/lume_spec.md 和 evaluator.rs 保持一致。
const keywords = ["fn", "ret", "if", "else", "while", "match", "case", "import", "and", "or", "not", "is", "true", "false", "end"];
const types = ["int", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "usize", "float", "f32", "f64", "string", "bool", "pointer", "unit", "void", "any"];

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

const keywordDocs: Record<string, string> = {
  fn: "declare a function",
  ret: "return from the current function",
  if: "conditional block or block expression",
  else: "alternate conditional branch",
  while: "loop block or block expression",
  match: "match expression",
  case: "match arm pattern",
  import: "import a relative Lume module",
  and: "short-circuit boolean and",
  or: "short-circuit boolean or",
  not: "boolean negation",
  is: "identity comparison",
  true: "boolean true",
  false: "boolean false",
  end: "close a block",
};

const keywordSnippets: Record<string, string> = {
  fn: "fn ${1:name}(${2:arg}:${3:int}) -> ${4:int}\n$0\nend",
  if: "if ${1:cond}\n$0\nend",
  while: "while ${1:cond}\n$0\nend",
  match: "match ${1:value}\ncase ${2:pattern}\n$0\nend",
};

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
      if (!/^fn\s+[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?(?:<[A-Za-z0-9_,\s]+>)?\s*\([^)]*\)(?:\s*->\s*[A-Za-z0-9_*]+)?\s*$/.test(line)) {
        diagnostics.push(new vscode.Diagnostic(
          new vscode.Range(i, 0, i, raw.length),
          "Invalid function declaration",
          vscode.DiagnosticSeverity.Error
        ));
      }
    }
    if (/^(?:if|while|match|fn)\b/.test(line)) blockDepth++;
    if (/=\s*(?:if|while|match)\b/.test(line)) blockDepth++;
    if (line === "end") {
      blockDepth--;
      if (blockDepth < 0) {
        diagnostics.push(new vscode.Diagnostic(
          new vscode.Range(i, 0, i, raw.length),
          "Unexpected 'end': no open block",
          vscode.DiagnosticSeverity.Error
        ));
        blockDepth = 0;
      }
    }
  }
  if (blockDepth > 0) {
    const i = document.lineCount - 1;
    diagnostics.push(new vscode.Diagnostic(
      new vscode.Range(i, 0, i, 0),
      `Unclosed block: expected ${blockDepth} 'end'`,
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

function wordPrefix(line: string, position: vscode.Position): { prefix: string; range?: vscode.Range } {
  const before = line.slice(0, position.character);
  const match = before.match(/[A-Za-z_][A-Za-z0-9_]*$/);
  if (!match) return { prefix: "" };
  const prefix = match[0];
  const start = position.character - prefix.length;
  return {
    prefix,
    range: new vscode.Range(position.line, start, position.line, position.character),
  };
}

function makeCompletion(
  label: string,
  kind: vscode.CompletionItemKind,
  range: vscode.Range | undefined,
  detail?: string,
  documentation?: string,
  insertText?: string | vscode.SnippetString
): vscode.CompletionItem {
  const item = new vscode.CompletionItem(label, kind);
  item.range = range;
  if (detail) item.detail = detail;
  if (documentation) item.documentation = documentation;
  if (insertText) item.insertText = insertText;
  return item;
}

function completionKind(word: string): vscode.CompletionItemKind {
  if (types.includes(word)) return vscode.CompletionItemKind.TypeParameter;
  if (builtinMap.has(word)) return vscode.CompletionItemKind.Function;
  return vscode.CompletionItemKind.Keyword;
}

function wordDescription(word: string): string {
  if (keywordDocs[word]) return keywordDocs[word];
  const builtin = builtinMap.get(word);
  if (builtin) return builtin.doc;
  return "Lume type";
}

export function activate(context: vscode.ExtensionContext) {
  const diagnostics = vscode.languages.createDiagnosticCollection("lume");
  context.subscriptions.push(diagnostics);
  const validate = (document: vscode.TextDocument) => {
    if (document.languageId === "lume") diagnostics.set(document.uri, diagnose(document));
  };
  context.subscriptions.push(vscode.workspace.onDidOpenTextDocument(validate));
  context.subscriptions.push(vscode.workspace.onDidSaveTextDocument(validate));
  context.subscriptions.push(vscode.workspace.onDidChangeTextDocument((event) => validate(event.document)));
  vscode.workspace.textDocuments.forEach(validate);

  context.subscriptions.push(vscode.languages.registerCompletionItemProvider(
    { language: "lume" },
    {
      provideCompletionItems(document, position) {
        const line = document.lineAt(position).text;
        const { prefix, range } = wordPrefix(line, position);
        const items: vscode.CompletionItem[] = [];
        const matches = (word: string) => !prefix || word.startsWith(prefix);

        for (const word of keywords) {
          if (!matches(word)) continue;
          const snippet = keywordSnippets[word];
          items.push(makeCompletion(
            word,
            vscode.CompletionItemKind.Keyword,
            range,
            `keyword · ${keywordDocs[word] ?? word}`,
            keywordDocs[word],
            snippet ? new vscode.SnippetString(snippet) : undefined
          ));
        }

        for (const word of types) {
          if (matches(word)) {
            items.push(makeCompletion(
              word,
              vscode.CompletionItemKind.TypeParameter,
              range,
              "Lume type",
              `Built-in type \`${word}\`.`
            ));
          }
        }

        for (const builtin of builtins) {
          if (!matches(builtin.name)) continue;
          items.push(makeCompletion(
            builtin.name,
            vscode.CompletionItemKind.Function,
            range,
            builtin.signature,
            builtin.doc,
            new vscode.SnippetString(`${builtin.name}($1)`)
          ));
        }

        const symbols = collectSymbols(document);
        for (const symbol of symbols.functions) {
          if (!matches(symbol.name)) continue;
          const detail = `fn ${symbol.name}(${symbol.params})${symbol.ret ? ` -> ${symbol.ret}` : ""}`;
          items.push(makeCompletion(
            symbol.name,
            vscode.CompletionItemKind.Function,
            range,
            detail,
            `Defined on line ${symbol.line + 1}`,
            new vscode.SnippetString(`${symbol.name}($1)`)
          ));
        }

        // 局部变量优先级最高，VS Code 会先显示 sortText 靠前的条目。
        for (const symbol of symbols.variables) {
          if (!matches(symbol.name)) continue;
          const item = makeCompletion(
            symbol.name,
            vscode.CompletionItemKind.Variable,
            range,
            symbol.type ? `variable: ${symbol.type}` : "variable",
            `Defined on line ${symbol.line + 1}`
          );
          item.sortText = `0_${symbol.name}`;
          items.push(item);
        }

        for (const item of items) {
          if (!item.sortText) item.sortText = `1_${item.label}`;
        }
        return items;
      }
    },
    ".", ":", "("
  ));

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

  context.subscriptions.push(vscode.languages.registerHoverProvider(
    { language: "lume" },
    {
      provideHover(document, position) {
        const range = document.getWordRangeAtPosition(position);
        if (!range) return;
        const word = document.getText(range);
        if ([...keywords, ...types].includes(word)) {
          return new vscode.Hover(`**${word}** · ${wordDescription(word)}`);
        }
        const builtin = builtinMap.get(word);
        if (builtin) {
          return new vscode.Hover(`\`${builtin.signature}\`\n\n${builtin.doc}`);
        }
        for (const symbol of collectSymbols(document).functions) {
          if (symbol.name === word) {
            return new vscode.Hover(
              `\`\`\`lume\nfn ${symbol.name}(${symbol.params})${symbol.ret ? ` -> ${symbol.ret}` : ""}\n\`\`\`\n\nDefined on line ${symbol.line + 1}`
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
