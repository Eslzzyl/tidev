import { useState, useCallback, useEffect, useRef, memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import "katex/dist/katex.min.css";
import { CopyButton } from "../ui/CopyButton";
import { Check, Copy, Download, Maximize2, Minimize2 } from "lucide-react";

// Dynamically import mermaid to avoid issues
let mermaidInstance: any = null;
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
  const [expanded, setExpanded] = useState(false);

  if (!props.src) return null;

  return (
    <span className="relative inline-block max-w-full">
      <button
        onClick={() => setExpanded(!expanded)}
        className="group relative block"
        title={expanded ? "Collapse" : "Expand image"}
      >
        <img
          {...props}
          className={`max-w-full rounded border border-neutral-200 dark:border-neutral-700 ${
            expanded ? "" : "max-h-96 object-contain"
          }`}
          loading="lazy"
        />
        <span className="absolute right-1 top-1 rounded bg-black/50 p-1 text-white opacity-0 transition-opacity group-hover:opacity-100">
          {expanded ? (
            <Minimize2 className="h-3 w-3" />
          ) : (
            <Maximize2 className="h-3 w-3" />
          )}
        </span>
      </button>
    </span>
  );
}

/**
 * Enhanced code block with language label, copy, and download buttons.
 */
function CodeBlock({
  language,
  children,
}: {
  language: string;
  children: string;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(children);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [children]);

  const handleDownload = useCallback(() => {
    const ext = languageToExtension(language);
    const blob = new Blob([children], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `code.${ext}`;
    a.click();
    URL.revokeObjectURL(url);
  }, [children, language]);

  return (
    <div className="group relative my-3 overflow-hidden rounded-lg border border-neutral-200 dark:border-neutral-700">
      {/* Header with language label and actions */}
      <div className="flex items-center justify-between border-b border-neutral-200 bg-neutral-50 px-3 py-1.5 dark:border-neutral-700 dark:bg-neutral-900">
        <span className="text-[10px] font-medium uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
          {language || "code"}
        </span>
        <div className="flex items-center gap-1">
          <button
            onClick={handleCopy}
            className="rounded p-1 text-neutral-400 hover:bg-neutral-200 hover:text-neutral-600 dark:hover:bg-neutral-700 dark:hover:text-neutral-300"
            title="Copy code"
          >
            {copied ? (
              <Check className="h-3.5 w-3.5" />
            ) : (
              <Copy className="h-3.5 w-3.5" />
            )}
          </button>
          <button
            onClick={handleDownload}
            className="rounded p-1 text-neutral-400 hover:bg-neutral-200 hover:text-neutral-600 dark:hover:bg-neutral-700 dark:hover:text-neutral-300"
            title="Download code"
          >
            <Download className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      {/* Code content */}
      <pre className="overflow-x-auto p-4 text-sm leading-relaxed">
        <code className="font-mono text-neutral-800 dark:text-neutral-200">
          {children}
        </code>
      </pre>
    </div>
  );
}

/**
 * Mermaid diagram component using the mermaid library.
 */
function Mermaid({ chart }: { chart: string }) {
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
          setError(e instanceof Error ? e.message : "Failed to render diagram");
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
        <p className="mb-1 text-xs font-medium text-red-600 dark:text-red-400">
          Diagram rendering failed
        </p>
        <pre className="text-xs text-red-500 dark:text-red-300">{chart}</pre>
        <button
          onClick={() => setKey((k) => k + 1)}
          className="mt-2 text-xs text-blue-500 hover:underline"
        >
          Retry
        </button>
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

export const MarkdownRenderer = memo(function MarkdownRenderer({
  content,
}: Props) {
  if (!content) return null;

  return (
    <div className="markdown-body prose prose-sm dark:prose-invert max-w-none">
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeKatex]}
        components={{
          a: CustomLink,
          img: CustomImage,

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
                  className="rounded bg-neutral-100 px-1.5 py-0.5 text-sm font-mono text-pink-600 dark:bg-neutral-800 dark:text-pink-400"
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

function languageToExtension(language: string): string {
  const map: Record<string, string> = {
    typescript: "ts",
    javascript: "js",
    python: "py",
    rust: "rs",
    go: "go",
    ruby: "rb",
    java: "java",
    c: "c",
    cpp: "cpp",
    csharp: "cs",
    css: "css",
    html: "html",
    json: "json",
    yaml: "yaml",
    yml: "yaml",
    toml: "toml",
    markdown: "md",
    bash: "sh",
    shell: "sh",
    sql: "sql",
    xml: "xml",
    php: "php",
    swift: "swift",
    kotlin: "kt",
    scala: "scala",
  };
  return map[language] || "txt";
}
