interface FenceRange {
  start: number;
  end: number;
}

const fenceStartPattern = /^ {0,3}(`{3,}|~{3,})[^\r\n]*(?:\r?\n|$)/gm;

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function isEscaped(value: string, index: number): boolean {
  let backslashCount = 0;
  for (let cursor = index - 1; cursor >= 0 && value[cursor] === "\\"; cursor -= 1) {
    backslashCount += 1;
  }
  return backslashCount % 2 === 1;
}

function skipCodeSpan(value: string, start: number): number | null {
  const runLength = (() => {
    let length = 0;
    while (value[start + length] === "`") length += 1;
    return length;
  })();
  if (runLength === 0) return null;

  const marker = "`".repeat(runLength);
  const end = value.indexOf(marker, start + runLength);
  return end === -1 ? null : end + runLength;
}

function findMathClose(value: string, start: number, close: string): number | null {
  let cursor = start;
  while (cursor < value.length) {
    if (value[cursor] === "`") {
      const codeEnd = skipCodeSpan(value, cursor);
      if (codeEnd !== null) {
        cursor = codeEnd;
        continue;
      }
    }

    if (value.startsWith(close, cursor) && !isEscaped(value, cursor)) {
      return cursor;
    }
    cursor += 1;
  }
  return null;
}

function normalizeMathSegment(segment: string): string {
  let result = "";
  let cursor = 0;

  while (cursor < segment.length) {
    if (segment[cursor] === "`") {
      const codeEnd = skipCodeSpan(segment, cursor);
      if (codeEnd !== null) {
        result += segment.slice(cursor, codeEnd);
        cursor = codeEnd;
        continue;
      }
    }

    const opener = segment.startsWith("\\[", cursor)
      ? { close: "\\]", display: true, replacement: "$$" }
      : segment.startsWith("\\(", cursor)
        ? { close: "\\)", display: false, replacement: "$" }
        : null;

    if (opener && !isEscaped(segment, cursor)) {
      const closeIndex = findMathClose(segment, cursor + 2, opener.close);
      if (closeIndex !== null) {
        result += opener.replacement;
        const body = segment.slice(cursor + 2, closeIndex);
        if (opener.display && body && !body.startsWith("\n") && !body.startsWith("\r")) {
          result += "\n";
        }
        result += body;
        if (opener.display && body && !body.endsWith("\n") && !body.endsWith("\r")) {
          result += "\n";
        }
        result += opener.replacement;
        cursor = closeIndex + 2;
        continue;
      }
    }

    result += segment[cursor];
    cursor += 1;
  }

  return result;
}

function findFenceRange(markdown: string, start: number, marker: string): FenceRange {
  const closingPattern = new RegExp(
    `^ {0,3}${escapeRegExp(marker[0])}{${marker.length},}[ \\t]*(?:\\r?\\n|$)`,
    "gm",
  );
  closingPattern.lastIndex = start;
  const closing = closingPattern.exec(markdown);
  return {
    start,
    end: closing ? closing.index + closing[0].length : markdown.length,
  };
}

/**
 * Normalize common LaTeX delimiters into the dollar-delimited syntax used by remark-math.
 * Markdown code spans and fenced code blocks are copied byte-for-byte.
 */
export function normalizeMathDelimiters(markdown: string): string {
  let result = "";
  let cursor = 0;
  const fences = [...markdown.matchAll(fenceStartPattern)];

  for (const match of fences) {
    const start = match.index ?? 0;
    if (start < cursor) continue;

    const range = findFenceRange(markdown, start, match[1]);
    result += normalizeMathSegment(markdown.slice(cursor, range.start));
    result += markdown.slice(range.start, range.end);
    cursor = range.end;
  }

  result += normalizeMathSegment(markdown.slice(cursor));
  return result;
}
