import { useState, useCallback, useEffect, useRef, memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import "katex/dist/katex.min.css";
import { Check, Code2, Copy, Maximize2, Minimize2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { HLJSApi } from "highlight.js";
import { Button, IconButton } from "../ui";

let hljsInstance: HLJSApi | null = null;
let hljsPromise: Promise<HLJSApi> | null = null;

function getHljs(): Promise<HLJSApi> {
  if (hljsInstance) return Promise.resolve(hljsInstance);
  if (!hljsPromise) {
    hljsPromise = import("highlight.js").then((mod) => {
      hljsInstance = mod.default;
      return hljsInstance;
    });
  }
  return hljsPromise;
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function highlightCode(code: string, language: string, hljs: HLJSApi | null): string {
  if (!hljs || !language) return escapeHtml(code);
  try {
    const lang = language.toLowerCase();
    if (hljs.getLanguage(lang)) {
      return hljs.highlight(code, { language: lang, ignoreIllegals: true }).value;
    }
  } catch {
    // Fall back to escaped text if highlight fails
  }
  return escapeHtml(code);
}

// Dynamically import mermaid to avoid issues
let mermaidInstance: {
  initialize: (config: Record<string, unknown>) => void;
  run: (config: Record<string, unknown>) => Promise<void>;
  render: (id: string, text: string) => Promise<{ svg: string }>;
} | null = null;
async function getMermaid() {
  if (!mermaidInstance) {
    const mermaidModule = await import("mermaid");
    mermaidInstance = mermaidModule.default || mermaidModule;
    mermaidInstance.initialize({
      startOnLoad: false,
      theme: "default",
      securityLevel: "loose",
    });
  }
  return mermaidInstance;
}

interface Props {
  content: string;
}

/**
 * Custom link component that opens in a new tab.
 */
function CustomLink(props: React.ComponentPropsWithoutRef<"a">) {
  return (
    <a
      {...props}
      target="_blank"
      rel="noopener noreferrer"
      className="text-blue-600 hover:text-blue-800 underline dark:text-blue-400 dark:hover:text-blue-300"
    />
  );
}

/**
 * Custom image component with click-to-expand.
 */
function CustomImage(props: React.ComponentPropsWithoutRef<"img">) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);

  if (!props.src) return null;

  return (
    <span className="relative inline-block max-w-full">
      <Button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="markdown-image-toggle"
        variant="ghost"
        size="sm"
        title={expanded ? t("Collapse") : t("Expand image")}
      >
        <img
          {...props}
          className={`max-w-full rounded border border-neutral-200 dark:border-neutral-700 ${
            expanded ? "" : "max-h-96 object-contain"
          }`}
          loading="lazy"
        />
        <span className="absolute right-1 top-1 rounded bg-black/50 p-1 text-white opacity-0 transition-opacity group-hover:opacity-100">
          {expanded ? <Minimize2 className="h-3 w-3" /> : <Maximize2 className="h-3 w-3" />}
        </span>
      </Button>
    </span>
  );
}

/**
 * Enhanced code block with syntax highlighting, language label, copy, and download buttons.
 */
function CodeBlock({ language, children }: { language: string; children: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const [highlightedHtml, setHighlightedHtml] = useState<string>(() =>
    highlightCode(children, language, hljsInstance),
  );

  useEffect(() => {
    let active = true;
    if (hljsInstance) {
      setHighlightedHtml(highlightCode(children, language, hljsInstance));
    } else {
      getHljs().then((hljs) => {
        if (active) {
          setHighlightedHtml(highlightCode(children, language, hljs));
        }
      });
    }
    return () => {
      active = false;
    };
  }, [children, language]);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(children);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [children]);

  return (
    <div className="markdown-code-card group relative my-3 overflow-hidden rounded-xl border border-neutral-200/80 bg-neutral-50/80 dark:border-neutral-800 dark:bg-[#0e131b]">
      {/* Header with icon, language label and actions */}
      <div className="markdown-code-header flex h-9 items-center justify-between border-b border-neutral-200/60 px-3.5 dark:border-neutral-800/80">
        <div className="flex items-center gap-1.5 text-xs text-neutral-500 dark:text-neutral-400">
          <Code2 className="h-3.5 w-3.5 opacity-70" />
          <span className="font-mono text-[11px] font-medium tracking-tight">
            {formatLanguageName(language)}
          </span>
        </div>
        <div className="flex items-center gap-1">
          <IconButton
            label={copied ? t("Copied!") : t("Copy code")}
            size="sm"
            variant={copied ? "primary" : "ghost"}
            type="button"
            onClick={handleCopy}
            title={copied ? t("Copied!") : t("Copy code")}
          >
            {copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
          </IconButton>
        </div>
      </div>

      {/* Code content */}
      <pre className="markdown-code-block m-0 overflow-x-auto p-4 font-mono text-[13px] leading-relaxed bg-transparent rounded-none">
        <code
          className="hljs font-mono text-neutral-800 dark:text-neutral-200"
          dangerouslySetInnerHTML={{ __html: highlightedHtml }}
        />
      </pre>
    </div>
  );
}

/**
 * Mermaid diagram component using the mermaid library.
 */
function Mermaid({ chart }: { chart: string }) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const [svg, setSvg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [key, setKey] = useState(0);

  useEffect(() => {
    let mounted = true;

    async function render() {
      try {
        const mermaid = await getMermaid();
        const id = `mermaid-${Math.random().toString(36).slice(2, 9)}`;
        const { svg: result } = await mermaid.render(id, chart);
        if (mounted) {
          setSvg(result);
          setError(null);
        }
      } catch (e) {
        if (mounted) {
          setError(e instanceof Error ? e.message : t("Failed to render diagram"));
          setSvg(null);
        }
      }
    }

    render();
    return () => {
      mounted = false;
    };
  }, [chart, key]);

  if (error) {
    return (
      <div className="my-3 rounded border border-red-200 bg-red-50 p-3 dark:border-red-800 dark:bg-red-950">
        <p className="markdown-error-label mb-1 text-red-600 dark:text-red-400">
          {t("Diagram rendering failed")}
        </p>
        <pre className="markdown-error-source text-red-500 dark:text-red-300">{chart}</pre>
        <Button
          type="button"
          onClick={() => setKey((k) => k + 1)}
          className="markdown-error-retry mt-2"
          variant="ghost"
          size="sm"
        >
          {t("Retry")}
        </Button>
      </div>
    );
  }

  if (!svg) {
    return (
      <div className="my-3 flex items-center justify-center rounded border border-neutral-200 p-8 dark:border-neutral-700">
        <div className="h-5 w-5 animate-spin rounded-full border-2 border-neutral-300 border-t-neutral-600" />
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      className="my-3 flex justify-center overflow-x-auto rounded border border-neutral-200 bg-white p-4 dark:border-neutral-700 dark:bg-neutral-900"
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}

/**
 * Detect if a code block is a Mermaid diagram.
 */
function isMermaidCode(language: string, content: string): boolean {
  return (
    language === "mermaid" ||
    content.trim().startsWith("graph ") ||
    content.trim().startsWith("sequenceDiagram") ||
    content.trim().startsWith("classDiagram") ||
    content.trim().startsWith("flowchart ") ||
    content.trim().startsWith("stateDiagram") ||
    content.trim().startsWith("gantt") ||
    content.trim().startsWith("pie ") ||
    content.trim().startsWith("erDiagram") ||
    content.trim().startsWith("journey") ||
    content.trim().startsWith("mindmap")
  );
}

export const MarkdownRenderer = memo(function MarkdownRenderer({ content }: Props) {
  if (!content) return null;

  return (
    <div className="markdown-body prose prose-sm dark:prose-invert max-w-none">
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeKatex]}
        components={{
          a: CustomLink,
          img: CustomImage,

          table({ children }) {
            return (
              <div className="markdown-table-scroll">
                <table>{children}</table>
              </div>
            );
          },

          // Enhanced code blocks
          code({ className, children, ...props }) {
            const match = /language-(\w+)/.exec(className || "");
            const language = match ? match[1] : "";
            const content = String(children).replace(/\n$/, "");

            // Check if it's a mermaid diagram
            if (isMermaidCode(language, content)) {
              return <Mermaid chart={content} />;
            }

            // Inline code (no language class and no newlines)
            if (!match && !content.includes("\n")) {
              return (
                <code
                  className="markdown-inline-code rounded bg-neutral-100 px-1.5 py-0.5 font-mono text-pink-600 dark:bg-neutral-800 dark:text-pink-400"
                  {...props}
                >
                  {children}
                </code>
              );
            }

            // Block code
            return <CodeBlock language={language}>{content}</CodeBlock>;
          },

          // Also handle pre > code pattern that react-markdown uses
          pre({ children }) {
            return <>{children}</>;
          },
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
});

function formatLanguageName(language: string): string {
  const map: Record<string, string> = {
    rust: "Rust",
    rs: "Rust",
    typescript: "TypeScript",
    ts: "TypeScript",
    tsx: "TSX",
    javascript: "JavaScript",
    js: "JavaScript",
    jsx: "JSX",
    python: "Python",
    py: "Python",
    go: "Go",
    ruby: "Ruby",
    rb: "Ruby",
    java: "Java",
    c: "C",
    cpp: "C++",
    "c++": "C++",
    csharp: "C#",
    "c#": "C#",
    cs: "C#",
    html: "HTML",
    css: "CSS",
    scss: "SCSS",
    sass: "Sass",
    less: "Less",
    json: "JSON",
    yaml: "YAML",
    yml: "YAML",
    toml: "TOML",
    sql: "SQL",
    sh: "Shell",
    bash: "Bash",
    zsh: "Zsh",
    shell: "Shell",
    markdown: "Markdown",
    md: "Markdown",
    xml: "XML",
    php: "PHP",
    swift: "Swift",
    kotlin: "Kotlin",
    kt: "Kotlin",
    scala: "Scala",
    lua: "Lua",
    dart: "Dart",
    zig: "Zig",
    diff: "Diff",
  };
  const key = language.trim().toLowerCase();
  return map[key] || (key ? key.charAt(0).toUpperCase() + key.slice(1) : "Code");
}
