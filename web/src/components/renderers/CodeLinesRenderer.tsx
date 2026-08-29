import { useCallback, useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { HLJSApi } from "highlight.js";

import { useChatScrollRef } from "../chat/ChatScrollContext";

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

const CODE_LINE_HEIGHT = 22;
const CODE_VIEWPORT_HEIGHT = 640;

function extractPath(output: string): string | null {
  const match = output.match(/<path>(.*?)<\/path>/);
  return match ? match[1] : null;
}

function parseContentLines(output: string): CodeLine[] {
  const contentMatch = output.match(/<content>\n?([\s\S]*?)\n?<\/content>/);
  const body = contentMatch ? contentMatch[1] : output;
  const lines: CodeLine[] = [];
  const linePattern = /^(\d+):\s?(.*)$/;

  for (const raw of body.split("\n")) {
    const match = raw.match(linePattern);
    if (match) lines.push({ lineNum: Number(match[1]), text: match[2] });
  }

  return lines;
}

function detectLanguage(filepath: string): string {
  const ext = filepath.split(".").pop()?.toLowerCase() || "";
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
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function useScrollMargin(
  scrollRef: ReturnType<typeof useChatScrollRef>,
  containerRef: RefObject<HTMLDivElement | null>,
) {
  const [scrollMargin, setScrollMargin] = useState(0);

  useLayoutEffect(() => {
    const scrollElement = scrollRef?.current;
    const container = containerRef.current;
    if (!scrollElement || !container) return;

    const update = () => {
      const scrollRect = scrollElement.getBoundingClientRect();
      const containerRect = container.getBoundingClientRect();
      const next = containerRect.top - scrollRect.top + scrollElement.scrollTop;
      setScrollMargin((current) => (Math.abs(current - next) < 0.5 ? current : next));
    };

    update();
    const resizeObserver =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    resizeObserver?.observe(scrollElement);
    resizeObserver?.observe(container);
    window.addEventListener("resize", update);
    return () => {
      resizeObserver?.disconnect();
      window.removeEventListener("resize", update);
    };
  }, [containerRef, scrollRef]);

  return scrollMargin;
}

export function CodeLinesRenderer({ output, filepath }: Props) {
  const [hljs, setHljs] = useState<HLJSApi | null>(null);
  const externalScrollRef = useChatScrollRef();
  const localScrollRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    let active = true;
    void import("highlight.js").then((mod) => {
      if (active) setHljs(mod.default);
    });
    return () => {
      active = false;
    };
  }, []);

  const filepathValue = useMemo(() => filepath || extractPath(output), [output, filepath]);
  const codeLines = useMemo(() => parseContentLines(output), [output]);
  const language = useMemo(
    () => (filepathValue ? detectLanguage(filepathValue) : ""),
    [filepathValue],
  );
  const scrollMargin = useScrollMargin(externalScrollRef, containerRef);
  const virtualizer = useVirtualizer({
    count: codeLines.length,
    getScrollElement: () => externalScrollRef?.current ?? localScrollRef.current,
    estimateSize: () => CODE_LINE_HEIGHT,
    getItemKey: (index) => String(codeLines[index]?.lineNum ?? index),
    initialOffset: () => externalScrollRef?.current?.scrollTop ?? 0,
    overscan: 20,
    scrollMargin,
  });

  const highlightLine = useCallback(
    (line: string): string => {
      if (!language || !hljs) return escapeHtml(line);
      try {
        return hljs.highlight(line, { language, ignoreIllegals: true }).value;
      } catch {
        return escapeHtml(line);
      }
    },
    [hljs, language],
  );

  if (codeLines.length === 0) {
    return <pre className="tool-code-lines-fallback">{output}</pre>;
  }

  const viewportHeight = Math.min(
    CODE_VIEWPORT_HEIGHT,
    Math.max(CODE_LINE_HEIGHT, codeLines.length * CODE_LINE_HEIGHT),
  );
  const usesExternalScroll = externalScrollRef !== null;
  const totalHeight = virtualizer.getTotalSize();

  return (
    <div
      ref={usesExternalScroll ? containerRef : localScrollRef}
      className="tool-code-lines overflow-x-auto"
      style={{
        height: `${usesExternalScroll ? totalHeight : viewportHeight}px`,
        overflowY: usesExternalScroll ? "visible" : "auto",
      }}
    >
      <div
        style={{
          position: "relative",
          height: `${totalHeight}px`,
          minWidth: "max-content",
        }}
      >
        {virtualizer.getVirtualItems().map((virtualItem) => {
          const line = codeLines[virtualItem.index];
          if (!line) return null;
          return (
            <div
              className="tool-code-line-row group flex min-w-full"
              data-index={virtualItem.index}
              key={virtualItem.key}
              ref={virtualizer.measureElement}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                minHeight: `${CODE_LINE_HEIGHT}px`,
                transform: `translateY(${virtualItem.start - scrollMargin}px)`,
                width: "100%",
              }}
            >
              <span className="tool-code-line-number w-8 shrink-0 select-none px-2 text-right text-neutral-400 dark:text-neutral-500">
                {line.lineNum}
              </span>
              <code
                className="tool-code-line text-neutral-800 dark:text-neutral-200"
                dangerouslySetInnerHTML={{ __html: highlightLine(line.text) }}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}
