import {
  useState,
  useRef,
  useEffect,
  useMemo,
  createContext,
  useContext,
  useCallback,
  useLayoutEffect,
  type WheelEvent,
  type UIEvent,
  type RefObject,
} from "react";
import { ChevronDown, ChevronRight, Expand, Minus } from "lucide-react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslation } from "react-i18next";
import {
  highlightDiffHunks,
  type DiffSyntaxHunkInput,
  type DiffSyntaxHunkResult,
} from "../../lib/diffSyntax";
import { useChatScrollRef } from "../chat/ChatScrollContext";

interface Props {
  diff: string;
  filepath: string;
  /** When true, render in a more compact inline style for chat messages */
  compact?: boolean;
}

const WIDE_LAYOUT_THRESHOLD = 768;
const DIFF_ROW_HEIGHT = 22;
const DIFF_INTERNAL_VIEWPORT_HEIGHT = 640;
const DIFF_OVERSCAN = 20;
const DIFF_CHARACTER_WIDTH = 8;
const DIFF_CELL_GUTTER = 52;

interface RawChange {
  type: "context" | "add" | "del";
  content: string;
  oldLine: number | null;
  newLine: number | null;
}

interface AlignedRow {
  left: {
    type: "context" | "del" | "empty";
    content: string;
    lineNum: number | null;
  };
  right: {
    type: "context" | "add" | "empty";
    content: string;
    lineNum: number | null;
  };
}

interface AlignedHunk {
  oldStart: number;
  newStart: number;
  rows: AlignedRow[];
}

interface HighlightedRow extends AlignedRow {
  leftHtml: string;
  rightHtml: string;
}

interface DiffRowItem {
  key: string;
  row: HighlightedRow;
  hunkEnd: boolean;
}

interface DiffScrollContextValue {
  scrollRef: RefObject<HTMLElement | null>;
  contentRef: RefObject<HTMLElement | null>;
}

type DiffScrollRef =
  | RefObject<HTMLElement | null>
  | RefObject<HTMLDivElement | null>
  | null
  | undefined;

const DiffScrollContext = createContext<DiffScrollContextValue | null>(null);

export function DiffScrollProvider({
  scrollRef,
  contentRef,
  children,
}: DiffScrollContextValue & { children: React.ReactNode }) {
  return (
    <DiffScrollContext.Provider value={{ scrollRef, contentRef }}>
      {children}
    </DiffScrollContext.Provider>
  );
}

function useDiffScrollMargin(
  scrollRef: DiffScrollRef,
  contentRef: RefObject<HTMLElement | null> | undefined,
  containerRef: RefObject<HTMLDivElement | null>,
) {
  const [scrollMargin, setScrollMargin] = useState(0);

  useLayoutEffect(() => {
    const scrollElement = scrollRef?.current;
    const contentElement = contentRef?.current;
    const container = containerRef.current;
    if (!scrollElement || !container) return;

    const updateScrollMargin = () => {
      const scrollRect = scrollElement.getBoundingClientRect();
      const containerRect = container.getBoundingClientRect();
      const nextScrollMargin = containerRect.top - scrollRect.top + scrollElement.scrollTop;
      setScrollMargin((current) =>
        Math.abs(current - nextScrollMargin) < 0.5 ? current : nextScrollMargin,
      );
    };

    updateScrollMargin();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", updateScrollMargin);
      return () => window.removeEventListener("resize", updateScrollMargin);
    }

    const resizeObserver = new ResizeObserver(updateScrollMargin);
    resizeObserver.observe(scrollElement);
    if (contentElement) resizeObserver.observe(contentElement);
    resizeObserver.observe(container);
    window.addEventListener("resize", updateScrollMargin);

    return () => {
      resizeObserver.disconnect();
      window.removeEventListener("resize", updateScrollMargin);
    };
  }, [containerRef, contentRef, scrollRef]);

  return scrollMargin;
}

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
    hh: "cpp",
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

function visualLineLength(text: string): number {
  let length = 0;
  for (const character of text) {
    if (character === "\t") {
      length += 8;
    } else if (character.codePointAt(0)! > 0xff) {
      length += 2;
    } else {
      length++;
    }
  }
  return length;
}

/**
 * Parse a unified diff string into content-aligned hunks.
 *
 * Uses the same approach as OpenChamber's parseDiffToLines:
 * 1. Split changes into leftSide (context + del) and rightSide (context + add)
 * 2. Find alignment points by matching context lines (same content on both sides)
 * 3. Walk both sides simultaneously, using alignment points as anchors
 */
function parseDiffAligned(diffText: string): AlignedHunk[] {
  const hunks: AlignedHunk[] = [];
  const lines = diffText.split("\n");
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Skip headers
    if (
      line.startsWith("Index:") ||
      line.startsWith("===") ||
      line.startsWith("---") ||
      line.startsWith("+++")
    ) {
      i++;
      continue;
    }

    // Parse hunk header
    if (line.startsWith("@@")) {
      const match = line.match(/@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
      const oldStart = match ? parseInt(match[1], 10) : 0;
      const newStart = match ? parseInt(match[2], 10) : 0;

      // Collect raw changes for this hunk
      const changes: RawChange[] = [];
      let oldLineNum = oldStart;
      let newLineNum = newStart;
      let j = i + 1;

      while (
        j < lines.length &&
        !lines[j].startsWith("@@") &&
        !lines[j].startsWith("Index:") &&
        !lines[j].startsWith("===")
      ) {
        const cl = lines[j];
        if (cl.startsWith("+")) {
          changes.push({
            type: "add",
            content: cl.slice(1),
            oldLine: null,
            newLine: newLineNum,
          });
          newLineNum++;
        } else if (cl.startsWith("-")) {
          changes.push({
            type: "del",
            content: cl.slice(1),
            oldLine: oldLineNum,
            newLine: null,
          });
          oldLineNum++;
        } else if (cl.startsWith(" ")) {
          changes.push({
            type: "context",
            content: cl.slice(1),
            oldLine: oldLineNum,
            newLine: newLineNum,
          });
          oldLineNum++;
          newLineNum++;
        }
        j++;
      }
      i = j;

      // --- Content-based alignment ---
      // Split into left side (context + del) and right side (context + add)
      const leftSide: {
        type: "context" | "del";
        lineNum: number;
        content: string;
      }[] = [];
      const rightSide: {
        type: "context" | "add";
        lineNum: number;
        content: string;
      }[] = [];

      for (const ch of changes) {
        if (ch.type === "context") {
          leftSide.push({
            type: "context",
            lineNum: ch.oldLine!,
            content: ch.content,
          });
          rightSide.push({
            type: "context",
            lineNum: ch.newLine!,
            content: ch.content,
          });
        } else if (ch.type === "del") {
          leftSide.push({
            type: "del",
            lineNum: ch.oldLine!,
            content: ch.content,
          });
        } else if (ch.type === "add") {
          rightSide.push({
            type: "add",
            lineNum: ch.newLine!,
            content: ch.content,
          });
        }
      }

      // Find alignment points: context lines with the same content on both sides
      const alignmentPoints: { leftIdx: number; rightIdx: number }[] = [];
      const rightContextIndices = new Map<string, number[]>();
      const rightContextOffsets = new Map<string, number>();

      rightSide.forEach((right, rightIdx) => {
        if (right.type !== "context") return;
        const indices = rightContextIndices.get(right.content);
        if (indices) {
          indices.push(rightIdx);
        } else {
          rightContextIndices.set(right.content, [rightIdx]);
        }
      });

      for (let li = 0; li < leftSide.length; li++) {
        const l = leftSide[li];
        if (l.type !== "context") continue;

        const candidates = rightContextIndices.get(l.content);
        if (!candidates) continue;
        const offset = rightContextOffsets.get(l.content) ?? 0;
        const foundRi = candidates[offset];
        if (foundRi === undefined) continue;

        rightContextOffsets.set(l.content, offset + 1);
        alignmentPoints.push({ leftIdx: li, rightIdx: foundRi });
      }
      alignmentPoints.sort((a, b) => a.leftIdx - b.leftIdx);

      // Walk both sides using alignment points as anchors
      const rows: AlignedRow[] = [];
      let leftIdx = 0;
      let rightIdx = 0;
      let alignIdx = 0;

      while (leftIdx < leftSide.length || rightIdx < rightSide.length) {
        const nextAlign = alignIdx < alignmentPoints.length ? alignmentPoints[alignIdx] : null;

        if (nextAlign && leftIdx === nextAlign.leftIdx && rightIdx === nextAlign.rightIdx) {
          // Alignment point: both sides have this context line
          rows.push({
            left: {
              type: "context",
              content: leftSide[leftIdx].content,
              lineNum: leftSide[leftIdx].lineNum,
            },
            right: {
              type: "context",
              content: rightSide[rightIdx].content,
              lineNum: rightSide[rightIdx].lineNum,
            },
          });
          leftIdx++;
          rightIdx++;
          alignIdx++;
        } else {
          const needLeft = leftIdx < leftSide.length && (!nextAlign || leftIdx < nextAlign.leftIdx);
          const needRight =
            rightIdx < rightSide.length && (!nextAlign || rightIdx < nextAlign.rightIdx);

          if (needLeft && needRight) {
            // Both sides have content before next alignment — pair them 1:1
            rows.push({
              left: {
                type: leftSide[leftIdx].type,
                content: leftSide[leftIdx].content,
                lineNum: leftSide[leftIdx].lineNum,
              },
              right: {
                type: rightSide[rightIdx].type,
                content: rightSide[rightIdx].content,
                lineNum: rightSide[rightIdx].lineNum,
              },
            });
            leftIdx++;
            rightIdx++;
          } else if (needLeft) {
            // Only left has content — empty right
            rows.push({
              left: {
                type: leftSide[leftIdx].type,
                content: leftSide[leftIdx].content,
                lineNum: leftSide[leftIdx].lineNum,
              },
              right: { type: "empty", content: "", lineNum: null },
            });
            leftIdx++;
          } else if (needRight) {
            // Only right has content — empty left
            rows.push({
              left: { type: "empty", content: "", lineNum: null },
              right: {
                type: rightSide[rightIdx].type,
                content: rightSide[rightIdx].content,
                lineNum: rightSide[rightIdx].lineNum,
              },
            });
            rightIdx++;
          } else {
            // Should not reach here, but break to avoid infinite loop
            break;
          }
        }
      }

      hunks.push({ oldStart, newStart, rows });
    } else {
      i++;
    }
  }

  return hunks;
}

export function DiffRenderer({ diff, filepath, compact = false }: Props) {
  const [containerWidth, setContainerWidth] = useState(0);
  const [isVisible, setIsVisible] = useState(false);
  const [syntaxResult, setSyntaxResult] = useState<{
    diff: string;
    language: string;
    hunks: DiffSyntaxHunkResult[];
  } | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  const leftHorizontalScrollbarRef = useRef<HTMLDivElement>(null);
  const rightHorizontalScrollbarRef = useRef<HTMLDivElement>(null);
  const inlineHorizontalScrollbarRef = useRef<HTMLDivElement>(null);
  const suppressedHorizontalScrollRef = useRef<{
    pane: "left" | "right";
    scrollLeft: number;
  } | null>(null);
  const [leftHorizontalScrollLeft, setLeftHorizontalScrollLeft] = useState(0);
  const [rightHorizontalScrollLeft, setRightHorizontalScrollLeft] = useState(0);
  const [inlineHorizontalScrollLeft, setInlineHorizontalScrollLeft] = useState(0);
  const diffScrollContext = useContext(DiffScrollContext);
  const chatScrollRef = useChatScrollRef();
  const externalScrollRef = diffScrollContext?.scrollRef ?? chatScrollRef;
  const scrollMargin = useDiffScrollMargin(
    externalScrollRef,
    diffScrollContext?.contentRef,
    containerRef,
  );
  const usesExternalScroll = externalScrollRef !== null;
  const isWide = containerWidth >= WIDE_LAYOUT_THRESHOLD;

  const language = useMemo(() => detectLanguage(filepath), [filepath]);
  const hunks = useMemo(() => parseDiffAligned(diff), [diff]);
  const maxLineLengths = useMemo(
    () =>
      hunks.reduce(
        (maxLengths, hunk) => {
          for (const row of hunk.rows) {
            maxLengths.left = Math.max(maxLengths.left, visualLineLength(row.left.content));
            maxLengths.right = Math.max(maxLengths.right, visualLineLength(row.right.content));
          }
          return maxLengths;
        },
        { left: 0, right: 0 },
      ),
    [hunks],
  );
  const leftCodeContentWidth = Math.max(1, maxLineLengths.left * DIFF_CHARACTER_WIDTH);
  const rightCodeContentWidth = Math.max(1, maxLineLengths.right * DIFF_CHARACTER_WIDTH);
  const leftDiffContentWidth = leftCodeContentWidth + DIFF_CELL_GUTTER;
  const rightDiffContentWidth = rightCodeContentWidth + DIFF_CELL_GUTTER;
  const inlineDiffContentWidth = Math.max(leftDiffContentWidth, rightDiffContentWidth);
  const paneWidth = isWide ? containerWidth / 2 : containerWidth;
  const leftHorizontalScrollLimit = Math.max(0, leftDiffContentWidth - paneWidth);
  const rightHorizontalScrollLimit = Math.max(0, rightDiffContentWidth - paneWidth);
  const inlineHorizontalScrollLimit = Math.max(0, inlineDiffContentWidth - containerWidth);
  const hasLeftHorizontalOverflow = containerWidth > 0 && leftHorizontalScrollLimit > 0;
  const hasRightHorizontalOverflow = containerWidth > 0 && rightHorizontalScrollLimit > 0;
  const hasInlineHorizontalOverflow = containerWidth > 0 && inlineHorizontalScrollLimit > 0;
  const syntaxInputs = useMemo<DiffSyntaxHunkInput[]>(
    () =>
      hunks.map((hunk, hunkIndex) => ({
        id: String(hunkIndex),
        leftLines: hunk.rows.map((row) => (row.left.type === "empty" ? "" : row.left.content)),
        rightLines: hunk.rows.map((row) => (row.right.type === "empty" ? "" : row.right.content)),
      })),
    [hunks],
  );

  useEffect(() => {
    if (!isVisible || !language || syntaxInputs.length === 0) return;
    if (syntaxResult?.diff === diff && syntaxResult.language === language) return;

    let active = true;
    highlightDiffHunks(language, syntaxInputs)
      .then((hunks) => {
        if (active) setSyntaxResult({ diff, language, hunks });
      })
      .catch(() => {
        if (active) setSyntaxResult({ diff, language, hunks: [] });
      });

    return () => {
      active = false;
    };
  }, [diff, isVisible, language, syntaxInputs, syntaxResult]);

  const rows = useMemo<DiffRowItem[]>(
    () =>
      hunks.flatMap((hunk, hunkIndex) => {
        const syntaxHunk =
          syntaxResult?.diff === diff && syntaxResult.language === language
            ? syntaxResult.hunks[hunkIndex]
            : undefined;

        return hunk.rows.map((row, rowIndex) => ({
          key: `${hunkIndex}-${rowIndex}`,
          row: {
            ...row,
            leftHtml: syntaxHunk?.leftHtml[rowIndex] ?? escapeHtml(row.left.content),
            rightHtml: syntaxHunk?.rightHtml[rowIndex] ?? escapeHtml(row.right.content),
          },
          hunkEnd: rowIndex === hunk.rows.length - 1,
        }));
      }),
    [diff, hunks, language, syntaxResult],
  );

  const rowVirtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => externalScrollRef?.current ?? viewportRef.current,
    estimateSize: (index) => (rows[index]?.hunkEnd ? DIFF_ROW_HEIGHT + 1 : DIFF_ROW_HEIGHT),
    getItemKey: (index) => rows[index]?.key ?? String(index),
    initialOffset: () => externalScrollRef?.current?.scrollTop ?? 0,
    overscan: DIFF_OVERSCAN,
    scrollMargin,
  });

  useEffect(() => {
    const element = containerRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;

    const observer = new ResizeObserver(([entry]) => {
      if (entry) setContainerWidth(entry.contentRect.width);
    });
    observer.observe(element);
    setContainerWidth(element.getBoundingClientRect().width);

    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const leftElement = leftHorizontalScrollbarRef.current;
    const rightElement = rightHorizontalScrollbarRef.current;
    const inlineElement = inlineHorizontalScrollbarRef.current;
    const leftScrollLeft = Math.min(leftElement?.scrollLeft ?? 0, leftHorizontalScrollLimit);
    const rightScrollLeft = Math.min(rightElement?.scrollLeft ?? 0, rightHorizontalScrollLimit);
    const inlineScrollLeft = Math.min(inlineElement?.scrollLeft ?? 0, inlineHorizontalScrollLimit);

    if (leftElement && leftElement.scrollLeft !== leftScrollLeft) {
      leftElement.scrollLeft = leftScrollLeft;
    }
    if (rightElement && rightElement.scrollLeft !== rightScrollLeft) {
      rightElement.scrollLeft = rightScrollLeft;
    }
    if (inlineElement && inlineElement.scrollLeft !== inlineScrollLeft) {
      inlineElement.scrollLeft = inlineScrollLeft;
    }

    setLeftHorizontalScrollLeft((current) =>
      current === leftScrollLeft ? current : leftScrollLeft,
    );
    setRightHorizontalScrollLeft((current) =>
      current === rightScrollLeft ? current : rightScrollLeft,
    );
    setInlineHorizontalScrollLeft((current) =>
      current === inlineScrollLeft ? current : inlineScrollLeft,
    );
  }, [leftHorizontalScrollLimit, rightHorizontalScrollLimit, inlineHorizontalScrollLimit]);

  useEffect(() => {
    const element = viewportRef.current;
    if (!element) return;
    if (typeof IntersectionObserver === "undefined") {
      setIsVisible(true);
      return;
    }

    const observer = new IntersectionObserver(([entry]) => setIsVisible(entry.isIntersecting), {
      rootMargin: "800px",
    });
    observer.observe(element);

    // Collapsible parents can animate from zero height without causing the
    // intersection observer to recalculate the viewport target. Re-observe
    // the target whenever its own or its container's layout changes so the
    // first expansion can trigger highlighting immediately.
    const resizeObserver =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(() => {
            observer.unobserve(element);
            observer.observe(element);
          });
    resizeObserver?.observe(element);
    if (containerRef.current) resizeObserver?.observe(containerRef.current);

    return () => {
      resizeObserver?.disconnect();
      observer.disconnect();
    };
  }, []);

  const handleWheel = useCallback(
    (pane: "left" | "right" | "inline", event: WheelEvent<HTMLDivElement>) => {
      const delta = event.deltaX || (event.shiftKey ? event.deltaY : 0);
      if (delta === 0) return;

      const element =
        pane === "left"
          ? leftHorizontalScrollbarRef.current
          : pane === "right"
            ? rightHorizontalScrollbarRef.current
            : inlineHorizontalScrollbarRef.current;
      const limit =
        pane === "left"
          ? leftHorizontalScrollLimit
          : pane === "right"
            ? rightHorizontalScrollLimit
            : inlineHorizontalScrollLimit;
      event.preventDefault();
      if (!element || limit === 0) return;
      element.scrollLeft = Math.max(0, Math.min(limit, element.scrollLeft + delta));
    },
    [leftHorizontalScrollLimit, rightHorizontalScrollLimit, inlineHorizontalScrollLimit],
  );

  const handleContainerWheel = useCallback(
    (event: WheelEvent<HTMLDivElement>) => {
      const delta = event.deltaX || (event.shiftKey ? event.deltaY : 0);
      if (delta === 0) return;

      const target = event.target instanceof HTMLElement ? event.target : null;
      const pane = target?.closest<HTMLElement>("[data-diff-pane]")?.dataset.diffPane as
        | "left"
        | "right"
        | "inline"
        | undefined;
      if (pane) {
        handleWheel(pane, event);
        return;
      }

      if (!isWide) {
        handleWheel("inline", event);
        return;
      }

      const bounds = containerRef.current?.getBoundingClientRect();
      const midpoint = bounds ? bounds.left + bounds.width / 2 : event.clientX;
      handleWheel(event.clientX < midpoint ? "left" : "right", event);
    },
    [handleWheel, isWide],
  );

  const syncHorizontalScroll = useCallback(
    (pane: "left" | "right", event: UIEvent<HTMLDivElement>) => {
      const value = event.currentTarget.scrollLeft;
      const suppressed = suppressedHorizontalScrollRef.current;
      if (suppressed && suppressed.pane === pane && suppressed.scrollLeft === value) {
        suppressedHorizontalScrollRef.current = null;
        return;
      }
      if (suppressed) suppressedHorizontalScrollRef.current = null;

      const otherPane = pane === "left" ? "right" : "left";
      const otherElement =
        otherPane === "left"
          ? leftHorizontalScrollbarRef.current
          : rightHorizontalScrollbarRef.current;
      const otherLimit =
        otherPane === "left" ? leftHorizontalScrollLimit : rightHorizontalScrollLimit;
      const otherValue = Math.min(value, otherLimit);

      if (otherElement && otherElement.scrollLeft !== otherValue) {
        suppressedHorizontalScrollRef.current = {
          pane: otherPane,
          scrollLeft: otherValue,
        };
        otherElement.scrollLeft = otherValue;
      }

      if (pane === "left") {
        setLeftHorizontalScrollLeft(value);
        setRightHorizontalScrollLeft(otherValue);
      } else {
        setRightHorizontalScrollLeft(value);
        setLeftHorizontalScrollLeft(otherValue);
      }
    },
    [leftHorizontalScrollLimit, rightHorizontalScrollLimit],
  );

  const renderInlineLine = (row: HighlightedRow) => {
    // For inline mode, prefer the side with content (del or add over empty)
    const hasDel = row.left.type === "del";
    const hasAdd = row.right.type === "add";
    const isContext = row.left.type === "context";

    if (isContext) {
      return (
        <div className="flex min-h-[22px] font-mono text-xs leading-[22px]" data-diff-pane="inline">
          <span className="w-8 shrink-0 select-none text-right text-neutral-400">
            {row.left.lineNum}
          </span>
          <span className="w-4 shrink-0 select-none text-center text-neutral-400"> </span>
          <span className="min-w-0 flex-1 overflow-hidden">
            <span
              className="block shrink-0 whitespace-pre pl-1 text-neutral-800 dark:text-neutral-200"
              style={{
                width: leftCodeContentWidth,
                transform: `translateX(-${inlineHorizontalScrollLeft}px)`,
              }}
              dangerouslySetInnerHTML={{ __html: row.leftHtml }}
            />
          </span>
        </div>
      );
    }

    if (hasDel) {
      return (
        <div
          className="flex min-h-[22px] font-mono text-xs leading-[22px] bg-red-50 dark:bg-red-950/40"
          data-diff-pane="inline"
        >
          <span className="w-8 shrink-0 select-none text-right text-neutral-400">
            {row.left.lineNum}
          </span>
          <span className="w-4 shrink-0 select-none text-center text-red-500">-</span>
          <span className="min-w-0 flex-1 overflow-hidden">
            <span
              className="block shrink-0 whitespace-pre pl-1 text-neutral-800 dark:text-neutral-200"
              style={{
                width: leftCodeContentWidth,
                transform: `translateX(-${inlineHorizontalScrollLeft}px)`,
              }}
              dangerouslySetInnerHTML={{ __html: row.leftHtml }}
            />
          </span>
        </div>
      );
    }

    if (hasAdd) {
      return (
        <div
          className="flex min-h-[22px] font-mono text-xs leading-[22px] bg-green-50 dark:bg-green-950/40"
          data-diff-pane="inline"
        >
          <span className="w-8 shrink-0 select-none text-right text-neutral-400">
            {row.right.lineNum}
          </span>
          <span className="w-4 shrink-0 select-none text-center text-green-600">+</span>
          <span className="min-w-0 flex-1 overflow-hidden">
            <span
              className="block shrink-0 whitespace-pre pl-1 text-neutral-800 dark:text-neutral-200"
              style={{
                width: rightCodeContentWidth,
                transform: `translateX(-${inlineHorizontalScrollLeft}px)`,
              }}
              dangerouslySetInnerHTML={{ __html: row.rightHtml }}
            />
          </span>
        </div>
      );
    }

    return null;
  };

  const renderTwoColumnLine = (row: HighlightedRow) => (
    <div className="flex border-b border-neutral-100 dark:border-neutral-800">
      {/* Left side (old) */}
      <div
        className={`flex min-h-[22px] min-w-0 flex-1 overflow-hidden font-mono text-xs leading-[22px] border-r border-neutral-200 dark:border-neutral-700 ${
          row.left.type === "del" ? "bg-red-50 dark:bg-red-950/40" : ""
        }`}
        data-diff-pane="left"
      >
        <span className="w-8 shrink-0 select-none text-right text-neutral-400">
          {row.left.lineNum ?? ""}
        </span>
        <span
          className={`w-4 shrink-0 select-none text-center ${
            row.left.type === "del" ? "text-red-500" : "text-neutral-400"
          }`}
        >
          {row.left.type === "del" ? "-" : row.left.type === "empty" ? "" : " "}
        </span>
        <span className="min-w-0 flex-1 overflow-hidden">
          <span
            className="block shrink-0 whitespace-pre pl-1 text-neutral-800 dark:text-neutral-200"
            style={{
              width: leftCodeContentWidth,
              transform: `translateX(-${leftHorizontalScrollLeft}px)`,
            }}
            dangerouslySetInnerHTML={{ __html: row.leftHtml }}
          />
        </span>
      </div>

      {/* Right side (new) */}
      <div
        className={`flex min-h-[22px] min-w-0 flex-1 overflow-hidden font-mono text-xs leading-[22px] ${
          row.right.type === "add" ? "bg-green-50 dark:bg-green-950/40" : ""
        }`}
        data-diff-pane="right"
      >
        <span className="w-8 shrink-0 select-none text-right text-neutral-400">
          {row.right.lineNum ?? ""}
        </span>
        <span
          className={`w-4 shrink-0 select-none text-center ${
            row.right.type === "add" ? "text-green-600" : "text-neutral-400"
          }`}
        >
          {row.right.type === "add" ? "+" : row.right.type === "empty" ? "" : " "}
        </span>
        <span className="min-w-0 flex-1 overflow-hidden">
          <span
            className="block shrink-0 whitespace-pre pl-1 text-neutral-800 dark:text-neutral-200"
            style={{
              width: rightCodeContentWidth,
              transform: `translateX(-${rightHorizontalScrollLeft}px)`,
            }}
            dangerouslySetInnerHTML={{ __html: row.rightHtml }}
          />
        </span>
      </div>
    </div>
  );

  if (!diff) return null;

  return (
    <div
      ref={containerRef}
      onWheel={handleContainerWheel}
      className={
        compact
          ? "tool-diff-renderer text-xs"
          : "rounded-lg border border-neutral-200 dark:border-neutral-800"
      }
      style={{ overscrollBehaviorX: "none" }}
    >
      <div
        ref={viewportRef}
        className="overflow-x-clip"
        style={{
          maxHeight: usesExternalScroll ? undefined : DIFF_INTERNAL_VIEWPORT_HEIGHT,
          overflowX: "clip",
          overflowY: usesExternalScroll ? "visible" : "auto",
          overscrollBehaviorX: "none",
        }}
      >
        <div className="relative" style={{ height: rowVirtualizer.getTotalSize() }}>
          {rowVirtualizer.getVirtualItems().map((virtualItem) => {
            const item = rows[virtualItem.index];
            if (!item) return null;
            return (
              <div
                key={item.key}
                data-index={virtualItem.index}
                className="absolute left-0 top-0 w-full"
                style={{ transform: `translateY(${virtualItem.start - scrollMargin}px)` }}
              >
                {isWide ? renderTwoColumnLine(item.row) : renderInlineLine(item.row)}
                {item.hunkEnd && <div className="h-px bg-neutral-200 dark:bg-neutral-800" />}
              </div>
            );
          })}
        </div>
      </div>
      {isWide ? (
        <div className="flex">
          <div
            ref={leftHorizontalScrollbarRef}
            className="w-1/2 overflow-x-auto overflow-y-hidden"
            onScroll={(event) => syncHorizontalScroll("left", event)}
            data-diff-pane="left"
            style={{ height: 12, overscrollBehaviorX: "none" }}
          >
            <div
              style={{
                width: hasLeftHorizontalOverflow ? leftDiffContentWidth : "100%",
                height: 1,
              }}
            />
          </div>
          <div
            ref={rightHorizontalScrollbarRef}
            className="w-1/2 overflow-x-auto overflow-y-hidden"
            onScroll={(event) => syncHorizontalScroll("right", event)}
            data-diff-pane="right"
            style={{ height: 12, overscrollBehaviorX: "none" }}
          >
            <div
              style={{
                width: hasRightHorizontalOverflow ? rightDiffContentWidth : "100%",
                height: 1,
              }}
            />
          </div>
        </div>
      ) : (
        hasInlineHorizontalOverflow && (
          <div
            ref={inlineHorizontalScrollbarRef}
            className="overflow-x-auto overflow-y-hidden"
            onScroll={(event) => setInlineHorizontalScrollLeft(event.currentTarget.scrollLeft)}
            data-diff-pane="inline"
            style={{ height: 12, overscrollBehaviorX: "none" }}
          >
            <div style={{ width: inlineDiffContentWidth, height: 1 }} />
          </div>
        )
      )}
    </div>
  );
}

/**
 * Collapsible diff file wrapper - shows file header with expand/collapse.
 */
export function CollapsibleDiffFile({
  diff,
  filepath,
  defaultExpanded = true,
  hideCollapseToggle = false,
}: Props & { defaultExpanded?: boolean; hideCollapseToggle?: boolean }) {
  const { t } = useTranslation();
  const { allExpanded } = useDiffCollapseContext();
  const [localExpanded, setLocalExpanded] = useState(defaultExpanded);

  // Sync with global collapse/expand all
  useEffect(() => {
    if (allExpanded !== null) {
      const id = requestAnimationFrame(() => setLocalExpanded(allExpanded));
      return () => cancelAnimationFrame(id);
    }
  }, [allExpanded]);

  const isExpanded = hideCollapseToggle ? true : localExpanded;
  const totalLines = useMemo(() => diff.split("\n").length, [diff]);

  return (
    <div className="overflow-hidden rounded-lg border border-neutral-200 dark:border-neutral-800">
      {/* File header */}
      <button
        onClick={() => !hideCollapseToggle && setLocalExpanded(!isExpanded)}
        className="flex w-full items-center gap-2 border-b border-neutral-200 bg-neutral-50 px-3 py-2 text-left text-xs font-medium text-neutral-700 transition-colors hover:bg-neutral-100 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-300 dark:hover:bg-neutral-800"
      >
        {hideCollapseToggle ? null : isExpanded ? (
          <ChevronDown className="h-3.5 w-3.5 text-neutral-400" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 text-neutral-400" />
        )}
        <span className="flex-1 truncate">{filepath}</span>
        <span className="text-[10px] text-neutral-400">
          {t("{{count}} lines", { count: totalLines })}
        </span>
      </button>

      {/* Diff content */}
      {isExpanded && <DiffRenderer diff={diff} filepath={filepath} compact />}
    </div>
  );
}

// Context for "expand/collapse all" control
interface DiffCollapseContextType {
  allExpanded: boolean | null;
  toggleAll: () => void;
}

const DiffCollapseContext = createContext<DiffCollapseContextType>({
  allExpanded: null,
  toggleAll: () => {},
});

// eslint-disable-next-line react-refresh/only-export-components
export function useDiffCollapseContext() {
  return useContext(DiffCollapseContext);
}

/**
 * Provider that manages "expand/collapse all" for multiple diff files.
 */
export function DiffCollapseProvider({ children }: { children: React.ReactNode }) {
  const { t } = useTranslation();
  const [allExpanded, setAllExpanded] = useState<boolean | null>(null);

  const toggleAll = useCallback(() => {
    setAllExpanded((prev) => (prev === null ? false : !prev));
  }, []);

  return (
    <DiffCollapseContext.Provider value={{ allExpanded, toggleAll }}>
      <div>
        <div className="mb-2 flex items-center justify-between">
          <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400">
            {t("File Changes")}
          </span>
          <button
            onClick={toggleAll}
            className="flex items-center gap-1 rounded px-2 py-1 text-[10px] text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800"
          >
            {allExpanded === false ? (
              <>
                <Expand className="h-3 w-3" /> {t("Expand all")}
              </>
            ) : (
              <>
                <Minus className="h-3 w-3" /> {t("Collapse all")}
              </>
            )}
          </button>
        </div>
        {children}
      </div>
    </DiffCollapseContext.Provider>
  );
}
