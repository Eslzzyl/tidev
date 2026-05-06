import { X, FileText, Copy, Check } from "lucide-react";
import { useState, useCallback } from "react";
import { useFileStore } from "../../stores/useFileStore";

export function CodeViewer() {
  const openFilePath = useFileStore((s) => s.openFilePath);
  const openFileContent = useFileStore((s) => s.openFileContent);
  const openFileLanguage = useFileStore((s) => s.openFileLanguage);
  const closeFile = useFileStore((s) => s.closeFile);
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(() => {
    if (openFileContent) {
      navigator.clipboard.writeText(openFileContent);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  }, [openFileContent]);

  if (!openFilePath) {
    return (
      <div className="flex h-full items-center justify-center text-xs text-neutral-400">
        Select a file to view
      </div>
    );
  }

  // Syntax highlighting HTML generation
  const highlightedContent = highlightCode(openFileContent || "", openFileLanguage || undefined);

  return (
    <div className="flex h-full flex-col">
      {/* Tab header */}
      <div className="flex items-center justify-between border-b border-neutral-200 bg-neutral-50 px-3 py-1.5 dark:border-neutral-800 dark:bg-neutral-900">
        <div className="flex min-w-0 items-center gap-2">
          <FileText className="h-4 w-4 shrink-0 text-neutral-400" />
          <span className="truncate text-xs font-medium text-neutral-700 dark:text-neutral-300">
            {openFilePath}
          </span>
          {openFileLanguage && (
            <span className="shrink-0 rounded bg-neutral-200 px-1.5 py-0.5 text-[10px] font-medium uppercase text-neutral-600 dark:bg-neutral-700 dark:text-neutral-400">
              {openFileLanguage}
            </span>
          )}
        </div>
        <div className="flex items-center gap-1">
          <button
            onClick={handleCopy}
            className="rounded p-1 text-neutral-400 hover:bg-neutral-200 hover:text-neutral-600 dark:hover:bg-neutral-700 dark:hover:text-neutral-300"
            aria-label="Copy file content"
          >
            {copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
          </button>
          <button
            onClick={closeFile}
            className="rounded p-1 text-neutral-400 hover:bg-neutral-200 hover:text-neutral-600 dark:hover:bg-neutral-700 dark:hover:text-neutral-300"
            aria-label="Close file"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      {/* Code content */}
      <div className="flex-1 overflow-auto">
        {openFileContent === null ? (
          <div className="flex h-full items-center justify-center">
            <div className="h-5 w-5 animate-spin rounded-full border-2 border-neutral-300 border-t-neutral-600" />
          </div>
        ) : (
          <pre className="p-4 text-xs leading-relaxed">
            <code
              className="font-mono text-neutral-800 dark:text-neutral-200"
              dangerouslySetInnerHTML={{ __html: highlightedContent }}
            />
          </pre>
        )}
      </div>
    </div>
  );
}

/**
 * Simple syntax highlighting using inline HTML spans.
 * This is a lightweight approach; for full IDE-like highlighting we'd use
 * CodeMirror or Monaco. This covers the most common languages.
 */
function highlightCode(code: string, language?: string): string {
  if (!language || !code) {
    return escapeHtml(code);
  }

  const rules = getHighlightRules(language);
  if (!rules) {
    return escapeHtml(code);
  }

  return applyHighlighting(code, rules);
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

type HighlightRule = [RegExp, string]; // [pattern, css class]

function applyHighlighting(code: string, rules: HighlightRule[]): string {
  // Tokenize by lines for simplicity
  const lines = code.split("\n");
  return lines
    .map((line) => {
      let result = escapeHtml(line);

      // Apply rules in order, wrapping matched parts in spans
      for (const [pattern, className] of rules) {
        result = result.replace(
          new RegExp(pattern.source, "g"),
          (match) => `<span class="${className}">${match}</span>`,
        );
      }

      return result;
    })
    .join("<br>");
}

function getHighlightRules(language: string): HighlightRule[] | null {
  const stringRule: HighlightRule = [/"[^"]*"/, "hljs-string"];
  const singleQuoteRule: HighlightRule = [/'[^']*'/, "hljs-string"];

  const common: HighlightRule[] = [
    [/\/\/.*$/, "hljs-comment"],
    [/\/\*[\s\S]*?\*\//, "hljs-comment"],
    stringRule,
    singleQuoteRule,
    [/`[^`]*`/, "hljs-string"],
    [/\b\d+\.?\d*\b/, "hljs-number"],
  ];

  const languageRules: Record<string, HighlightRule[]> = {
    typescript: [
      ...common,
      [/\b(import|export|from|const|let|var|function|return|if|else|for|while|class|interface|type|extends|implements|async|await|new|this|super|typeof|instanceof|keyof|readonly|enum|namespace|module|declare|abstract|private|protected|public|static)\b/, "hljs-keyword"],
      [/\b(string|number|boolean|void|never|any|unknown|undefined|null|object|symbol|bigint|Promise|Record|Partial|Required|Pick|Omit|Exclude|Extract)\b/, "hljs-type"],
    ],
    javascript: [
      ...common,
      [/\b(import|export|from|const|let|var|function|return|if|else|for|while|class|extends|async|await|new|this|super|typeof|instanceof|delete|try|catch|throw|yield)\b/, "hljs-keyword"],
    ],
    rust: [
      ...common,
      [/\b(fn|let|mut|const|pub|use|mod|struct|enum|impl|trait|return|if|else|for|while|loop|match|async|await|unsafe|ref|move|where|as|in|self|super|crate|type|dyn|impl|default|union|static|extern|macro_rules)\b/, "hljs-keyword"],
      [/\b(u8|u16|u32|u64|u128|i8|i16|i32|i64|i128|f32|f64|bool|char|str|String|Vec|Option|Result|Box|Rc|Arc|HashMap|HashSet|Mutex|Cell|RefCell)\b/, "hljs-type"],
      [/::/, "hljs-operator"],
      [/->/, "hljs-operator"],
      [/=>/, "hljs-operator"],
    ],
    python: [
      [/#.*$/, "hljs-comment"],
      stringRule,
      singleQuoteRule,
      [/""".*?"""/, "hljs-string"],
      [/\b\d+\.?\d*\b/, "hljs-number"],
      [/\b(def|class|return|if|elif|else|for|while|import|from|as|with|try|except|finally|raise|yield|lambda|pass|break|continue|and|or|not|is|in|async|await|self|None|True|False|print|range|len|type|super|del|global|nonlocal)\b/, "hljs-keyword"],
    ],
    go: [
      ...common,
      [/\b(func|package|import|return|if|else|for|range|switch|case|default|break|continue|go|defer|select|chan|map|struct|interface|type|var|const|nil|true|false|make|new|append|len|cap|close|panic|recover|fallthrough)\b/, "hljs-keyword"],
    ],
    css: [
      [/\/\*[\s\S]*?\*\//, "hljs-comment"],
      [/#[a-zA-Z0-9_-]+/, "hljs-selector-id"],
      [/\.[a-zA-Z0-9_-]+/, "hljs-selector-class"],
      [/@[a-zA-Z-]+/, "hljs-keyword"],
      [/\b([a-zA-Z-]+)\s*:/, "hljs-attribute"],
      stringRule,
      [/\b(\d+)(px|em|rem|%|vh|vw|pt)?\b/, "hljs-number"],
    ],
    json: [
      [/"([^"\\]|\\.)*"\s*:/, "hljs-attr"],
      stringRule,
      [/\b(true|false|null)\b/, "hljs-literal"],
      [/\b\d+\.?\d*\b/, "hljs-number"],
    ],
  };

  return languageRules[language] || null;
}