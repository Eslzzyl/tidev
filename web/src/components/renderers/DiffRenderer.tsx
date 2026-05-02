import { useState, useRef, useEffect, useMemo } from 'react';
import { parsePatch } from 'diff';
import hljs from 'highlight.js';

interface Props {
  diff: string;
  filepath: string;
}

const WIDE_LAYOUT_THRESHOLD = 768;

interface DiffLine {
  type: 'context' | 'add' | 'del';
  content: string;
  oldLineNum: number | null;
  newLineNum: number | null;
}

interface AlignedRow {
  left: { type: 'context' | 'del' | 'empty'; content: string; lineNum: number | null } | null;
  right: { type: 'context' | 'add' | 'empty'; content: string; lineNum: number | null } | null;
}

function detectLanguage(fp: string): string {
  const ext = fp.split('.').pop()?.toLowerCase() || '';
  const langMap: Record<string, string> = {
    rs: 'rust', ts: 'typescript', tsx: 'tsx', js: 'javascript', jsx: 'jsx',
    py: 'python', go: 'go', rb: 'ruby', java: 'java', kt: 'kotlin',
    scala: 'scala', swift: 'swift', c: 'c', h: 'c', cpp: 'cpp', hpp: 'cpp',
    cc: 'cpp', hh: 'cpp', cxx: 'cpp', cs: 'csharp', php: 'php',
    html: 'html', css: 'css', scss: 'scss', sass: 'sass', less: 'less',
    sql: 'sql', sh: 'bash', bash: 'bash', zsh: 'bash', yaml: 'yaml',
    yml: 'yaml', toml: 'toml', json: 'json', xml: 'xml', md: 'markdown',
    mdx: 'markdown', svelte: 'svelte', vue: 'vue', lua: 'lua', dart: 'dart',
    r: 'r', zig: 'zig', nim: 'nim',
  };
  return langMap[ext] || '';
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function highlightLine(line: string, language: string): string {
  if (!language) return escapeHtml(line);
  try {
    const result = hljs.highlight(line, { language, ignoreIllegals: true });
    return result.value;
  } catch {
    return escapeHtml(line);
  }
}

function parseDiff(diffText: string): { hunks: DiffLine[][] } {
  const hunks: DiffLine[][] = [];

  try {
    const patches = parsePatch(diffText);
    for (const patch of patches) {
      const hunkLines: DiffLine[] = [];
      if (patch.hunks) {
        for (const hunk of patch.hunks) {
          let oldLine = hunk.oldStart;
          let newLine = hunk.newStart;

          for (const line of hunk.lines) {
            if (line.startsWith(' ')) {
              hunkLines.push({ type: 'context', content: line.slice(1), oldLineNum: oldLine, newLineNum: newLine });
              oldLine++;
              newLine++;
            } else if (line.startsWith('+')) {
              hunkLines.push({ type: 'add', content: line.slice(1), oldLineNum: null, newLineNum: newLine });
              newLine++;
            } else if (line.startsWith('-')) {
              hunkLines.push({ type: 'del', content: line.slice(1), oldLineNum: oldLine, newLineNum: null });
              oldLine++;
            }
          }
        }
      }
      if (hunkLines.length > 0) {
        hunks.push(hunkLines);
      }
    }
  } catch {
    // If parsing fails, treat the whole diff as a single hunk of additions
    hunks.push(
      diffText.split('\n').map((line, i) => ({
        type: 'add' as const,
        content: line,
        oldLineNum: null,
        newLineNum: i + 1,
      }))
    );
  }

  return { hunks };
}

function alignRows(hunk: DiffLine[]): AlignedRow[] {
  const rows: AlignedRow[] = [];
  let i = 0;

  while (i < hunk.length) {
    const line = hunk[i];

    if (line.type === 'context') {
      rows.push({
        left: { type: 'context', content: line.content, lineNum: line.oldLineNum },
        right: { type: 'context', content: line.content, lineNum: line.newLineNum },
      });
      i++;
    } else if (line.type === 'del') {
      const next = i + 1 < hunk.length ? hunk[i + 1] : null;
      if (next && next.type === 'add') {
        rows.push({
          left: { type: 'del', content: line.content, lineNum: line.oldLineNum },
          right: { type: 'add', content: next.content, lineNum: next.newLineNum },
        });
        i += 2;
      } else {
        rows.push({
          left: { type: 'del', content: line.content, lineNum: line.oldLineNum },
          right: { type: 'add', content: '', lineNum: null },
        });
        i++;
      }
    } else if (line.type === 'add') {
      rows.push({
        left: { type: 'empty', content: '', lineNum: null },
        right: { type: 'add', content: line.content, lineNum: line.newLineNum },
      });
      i++;
    }
  }

  return rows;
}

export function DiffRenderer({ diff, filepath }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [containerWidth, setContainerWidth] = useState(0);
  const isWide = containerWidth >= WIDE_LAYOUT_THRESHOLD;

  const language = useMemo(() => detectLanguage(filepath), [filepath]);
  const { hunks } = useMemo(() => parseDiff(diff), [diff]);

  useEffect(() => {
    if (containerRef.current) {
      setContainerWidth(containerRef.current.clientWidth);
    }
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        setContainerWidth(entry.contentRect.width);
      }
    });
    if (containerRef.current) {
      observer.observe(containerRef.current);
    }
    return () => observer.disconnect();
  }, []);

  const renderInlineLine = (line: DiffLine) => {
    const bgClass =
      line.type === 'add'
        ? 'bg-green-50 dark:bg-green-950/30'
        : line.type === 'del'
          ? 'bg-red-50 dark:bg-red-950/30'
          : '';

    return (
      <div key={`${line.type}-${line.oldLineNum ?? ''}-${line.newLineNum ?? ''}`} className={`flex ${bgClass}`}>
        <span className="diff-line-num select-none text-right text-neutral-400 dark:text-neutral-600 min-w-[3rem] px-2 text-xs leading-relaxed">
          {line.oldLineNum ?? ''}
        </span>
        <span className="diff-line-num select-none text-right text-neutral-400 dark:text-neutral-600 min-w-[3rem] px-2 text-xs leading-relaxed">
          {line.newLineNum ?? ''}
        </span>
        <span className="diff-line-prefix select-none w-4 text-center text-xs leading-relaxed text-neutral-400">
          {line.type === 'add' ? '+' : line.type === 'del' ? '-' : ' '}
        </span>
        <span
          className="flex-1 px-2 leading-relaxed font-mono text-xs whitespace-pre-wrap break-all"
          dangerouslySetInnerHTML={{
            __html: highlightLine(line.content, language),
          }}
        />
      </div>
    );
  };

  const renderTwoColumn = (aligned: AlignedRow[]) => {
    return (
      <div className="diff-two-column w-full overflow-x-auto">
        {aligned.map((row, idx) => (
          <div key={idx} className="flex">
            <div
              className={`w-1/2 flex ${row.left?.type === 'del' ? 'bg-red-50 dark:bg-red-950/30' : row.left?.type === 'context' ? '' : ''}`}
            >
              {row.left && (
                <>
                  <span className="diff-line-num select-none text-right text-neutral-400 dark:text-neutral-600 min-w-[3rem] px-2 text-xs leading-relaxed">
                    {row.left.lineNum ?? ''}
                  </span>
                  <span className="diff-line-prefix select-none w-4 text-center text-xs leading-relaxed text-neutral-400">
                    {row.left.type === 'del' ? '-' : ' '}
                  </span>
                  <span
                    className="flex-1 px-2 leading-relaxed font-mono text-xs whitespace-pre-wrap break-all"
                    dangerouslySetInnerHTML={{
                      __html: row.left.type !== 'empty' ? highlightLine(row.left.content, language) : '',
                    }}
                  />
                </>
              )}
            </div>
            <div
              className={`w-1/2 flex ${row.right?.type === 'add' ? 'bg-green-50 dark:bg-green-950/30' : row.right?.type === 'context' ? '' : ''}`}
            >
              {row.right && (
                <>
                  <span className="diff-line-num select-none text-right text-neutral-400 dark:text-neutral-600 min-w-[3rem] px-2 text-xs leading-relaxed">
                    {row.right.lineNum ?? ''}
                  </span>
                  <span className="diff-line-prefix select-none w-4 text-center text-xs leading-relaxed text-neutral-400">
                    {row.right.type === 'add' ? '+' : row.right.type === 'context' ? ' ' : ''}
                  </span>
                  <span
                    className="flex-1 px-2 leading-relaxed font-mono text-xs whitespace-pre-wrap break-all"
                    dangerouslySetInnerHTML={{
                      __html: row.right.type !== 'empty' ? highlightLine(row.right.content, language) : '',
                    }}
                  />
                </>
              )}
            </div>
          </div>
        ))}
      </div>
    );
  };

  return (
    <div ref={containerRef} className="diff-container overflow-hidden rounded-lg border border-neutral-200 dark:border-neutral-800">
      {isWide ? (
        hunks.map((hunk, idx) => (
          <div key={idx} className="border-b border-neutral-200 last:border-b-0 dark:border-neutral-800">
            {renderTwoColumn(alignRows(hunk))}
          </div>
        ))
      ) : (
        hunks.map((hunk, idx) => (
          <div key={idx} className="border-b border-neutral-200 last:border-b-0 dark:border-neutral-800">
            {hunk.map((line) => renderInlineLine(line))}
          </div>
        ))
      )}
    </div>
  );
}
