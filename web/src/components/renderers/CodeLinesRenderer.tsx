import { useMemo, useState, useEffect, useCallback } from "react";

interface CodeLine {
  lineNum: number;
  text: string;
}

interface Props {
  /**
   * Raw output string from the read tool, which includes an XML-style
   * <content>…</content> block with lines formatted as `lineNum: text`.
   */
  output: string;
  filepath?: string;
}

/**
 * Extract the file path from tool output XML.
 */
function extractPath(output: string): string | null {
  const m = output.match(/<path>(.*?)<\/path>/);
  return m ? m[1] : null;
}

/**
 * Parse lines from the <content>…</content> section.
 * Each line is expected to be `lineNum: text`.
 */
function parseContentLines(output: string): CodeLine[] {
  // Try to extract content between <content> and </content>
  const contentMatch = output.match(/<content>\n?([\s\S]*?)\n?<\/content>/);
  const body = contentMatch ? contentMatch[1] : output;

  const lines: CodeLine[] = [];
  const linePattern = /^(\d+):\s?(.*)$/m;

  for (const raw of body.split("\n")) {
    const m = raw.match(linePattern);
    if (m) {
      lines.push({ lineNum: parseInt(m[1], 10), text: m[2] });
    }
  }

  return lines;
}

/**
 * Detect the programming language from the file path.
 */
function detectLanguage(fp: string): string {
  const ext = fp.split(".").pop()?.toLowerCase() || "";
  const langMap: Record<string, string> = {
    rs: "rust",
    ts: "typescript",
    tsx: "tsx",
    js: "javascript",
    jsx: "jsx",
    py: "python",
    go: "go",
    rb: "ruby",
    java: "java",
    kt: "kotlin",
    scala: "scala",
    swift: "swift",
    c: "c",
    h: "c",
    cpp: "cpp",
    hpp: "cpp",
    cc: "cpp",
    cxx: "cpp",
    cs: "csharp",
    php: "php",
    html: "html",
    css: "css",
    scss: "scss",
    sass: "sass",
    less: "less",
    sql: "sql",
    sh: "bash",
    bash: "bash",
    zsh: "bash",
    yaml: "yaml",
    yml: "yaml",
    toml: "toml",
    json: "json",
    xml: "xml",
    md: "markdown",
    mdx: "markdown",
    svelte: "svelte",
    vue: "vue",
    lua: "lua",
    dart: "dart",
    r: "r",
    zig: "zig",
    nim: "nim",
  };
  return langMap[ext] || "";
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export function CodeLinesRenderer({ output, filepath }: Props) {
  const [hljs, setHljs] = useState<typeof import("highlight.js") | null>(null);

  // Dynamically load highlight.js on first render
  useEffect(() => {
    import("highlight.js").then((mod) => setHljs(() => mod.default));
  }, []);

  const highlightLine = useCallback(
    (line: string, language: string): string => {
      if (!language || !hljs) return escapeHtml(line);
      try {
        const result = hljs.highlight(line, { language, ignoreIllegals: true });
        return result.value;
      } catch {
        return escapeHtml(line);
      }
    },
    [hljs],
  );

  const fp = useMemo(() => filepath || extractPath(output), [output, filepath]);
  const codeLines = useMemo(() => parseContentLines(output), [output]);
  const language = useMemo(() => (fp ? detectLanguage(fp) : ""), [fp]);

  // Fallback: if no structured lines found, render as plain text
  if (codeLines.length === 0) {
    return (
      <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-xs leading-relaxed text-neutral-600 dark:text-neutral-400">
        {output}
      </pre>
    );
  }

  return (
    <div className="overflow-x-auto rounded-md border border-neutral-200 dark:border-neutral-700">
      <table className="w-full border-collapse text-xs leading-relaxed">
        <tbody>
          {codeLines.map(({ lineNum, text }) => (
            <tr key={lineNum} className="group">
              <td className="select-none border-r border-neutral-200 px-2 text-right text-neutral-400 dark:border-neutral-700 dark:text-neutral-500">
                {lineNum}
              </td>
              <td className="px-3 font-mono text-neutral-800 dark:text-neutral-200">
                <code
                  dangerouslySetInnerHTML={{
                    __html: highlightLine(text, language),
                  }}
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
