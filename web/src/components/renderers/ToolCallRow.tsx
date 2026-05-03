import { useState } from 'react';
import { Loader2, ChevronDown } from 'lucide-react';
import type { ToolCallEntry } from '../../types/round';
import { MarkdownRenderer } from './MarkdownRenderer';
import { DiffRenderer } from './DiffRenderer';

interface Props {
  entry: ToolCallEntry;
}

function isReadOnlyTool(name: string): boolean {
  return ['read', 'list', 'grep', 'glob', 'skill'].includes(name);
}

function isWriteTool(name: string): boolean {
  return ['write', 'edit', 'apply_patch'].includes(name);
}

function isBash(name: string): boolean {
  return name === 'bash';
}

function getToolIcon(name: string): string {
  switch (name) {
    case 'read':
      return 'M19.5 14.25v-2.625a3.375 3.375 0 0 0-3.375-3.375h-1.5A1.125 1.125 0 0 1 13.5 7.125v-1.5a3.375 3.375 0 0 0-3.375-3.375H8.25m0 12.75h7.5m-7.5 3H12M10.5 2.25H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 0 0-9-9Z';
    case 'list':
      return 'M2.25 12.75V12A2.25 2.25 0 0 1 4.5 9.75h15A2.25 2.25 0 0 1 21.75 12v.75m-8.69-6.44-2.12-2.12a1.5 1.5 0 0 0-1.061-.44H4.5A2.25 2.25 0 0 0 2.25 6v12a2.25 2.25 0 0 0 2.25 2.25h15A2.25 2.25 0 0 0 21.75 18V9a2.25 2.25 0 0 0-2.25-2.25h-5.379a1.5 1.5 0 0 1-1.06-.44Z';
    case 'grep':
      return 'm21 21-5.197-5.197m0 0A7.5 7.5 0 1 0 5.196 5.196a7.5 7.5 0 0 0 10.607 10.607Z';
    case 'glob':
      return 'M3.75 9.776c.112-.017.227-.026.344-.026h15.812c.117 0 .232.009.344.026m-16.5 0a2.25 2.25 0 0 0-1.883 2.542l.857 6a2.25 2.25 0 0 0 2.227 1.932H19.05a2.25 2.25 0 0 0 2.227-1.932l.857-6a2.25 2.25 0 0 0-1.883-2.542m-16.5 0V6A2.25 2.25 0 0 1 6 3.75h3.879a1.5 1.5 0 0 1 1.06.44l2.122 2.12a1.5 1.5 0 0 0 1.06.44H18A2.25 2.25 0 0 1 20.25 9v.776';
    case 'write':
    case 'edit':
      return 'm16.862 4.487 1.687-1.688a1.875 1.875 0 1 1 2.652 2.652L6.832 19.82a4.5 4.5 0 0 1-1.897 1.13l-2.685.8.8-2.685a4.5 4.5 0 0 1 1.13-1.897L16.863 4.487Zm0 0L19.5 7.125';
    case 'apply_patch':
      return 'M19.5 14.25v-2.625a3.375 3.375 0 0 0-3.375-3.375h-1.5A1.125 1.125 0 0 1 13.5 7.125v-1.5a3.375 3.375 0 0 0-3.375-3.375H8.25m3.75 9v6m3-3H9m1.5-12H5.625c-.621 0-1.125.5-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 0 0-9-9Z';
    case 'bash':
      return 'm6.75 7.5 3 2.25-3 2.25m4.5 0h3m-9 8.25h13.5A2.25 2.25 0 0 0 21 18V6a2.25 2.25 0 0 0-2.25-2.25H5.25A2.25 2.25 0 0 0 3 6v12a2.25 2.25 0 0 0 2.25 2.25Z';
    case 'skill':
      return 'M12 18v-5.25m0 0a6.01 6.01 0 0 0 1.5-.189m-1.5.189a6.01 6.01 0 0 1-1.5-.189m3.75 7.478a12.06 12.06 0 0 1-4.5 0m3.75 2.383a14.406 14.406 0 0 1-3 0M14.25 18v-.192c0-.983.658-1.823 1.508-2.316a7.5 7.5 0 1 0-7.517 0c.85.493 1.509 1.333 1.509 2.316V18';
    case 'task':
      return 'M2.375 6.75A2.25 2.25 0 0 1 4.625 4.5h14.75a2.25 2.25 0 0 1 2.25 2.25v10.5a2.25 2.25 0 0 1-2.25 2.25H4.625a2.25 2.25 0 0 1-2.25-2.25V6.75Zm16.5 0H5.125v4.5h13.75v-4.5Zm-13.75 6v3h13.75v-3H5.125Z';
    default:
      return 'M17.25 6.75 22.5 12l-5.25 5.25m-10.5 0L1.5 12l5.25-5.25m7.5-3-4.5 16.5';
  }
}

function getToolColor(name: string): string {
  if (isReadOnlyTool(name)) return 'text-blue-600 dark:text-blue-400';
  if (isWriteTool(name)) return 'text-emerald-600 dark:text-emerald-400';
  if (isBash(name)) return 'text-violet-600 dark:text-violet-400';
  if (name === 'task') return 'text-amber-600 dark:text-amber-400';
  return 'text-neutral-600 dark:text-neutral-400';
}

function getToolBg(name: string): string {
  if (isReadOnlyTool(name)) return 'bg-blue-50 dark:bg-blue-950/30 border-blue-200 dark:border-blue-800';
  if (isWriteTool(name)) return 'border-neutral-200 dark:border-neutral-700';
  if (isBash(name)) return 'bg-violet-50 dark:bg-violet-950/30 border-violet-200 dark:border-violet-800';
  if (name === 'task') return 'bg-amber-50 dark:bg-amber-950/30 border-amber-200 dark:border-amber-800';
  return 'bg-neutral-50 dark:bg-neutral-800 border-neutral-200 dark:border-neutral-700';
}

function summarizeArguments(name: string, entry: ToolCallEntry): string {
  try {
    const args = JSON.parse(entry.arguments);
    switch (name) {
      case 'read':
      case 'write':
      case 'edit':
      case 'list': {
        return args.path || '(unknown)';
      }
      case 'grep': {
        const pattern = args.pattern || '';
        const path = args.path || '.';
        return pattern ? `"${pattern}" in ${path}` : path;
      }
      case 'glob': {
        const pattern = args.pattern || '*';
        return `${pattern}`;
      }
      case 'bash': {
        return args.command || '(no command)';
      }
      case 'apply_patch': {
        return args.path || '(unknown)';
      }
      case 'skill': {
        return args.name || '(unknown)';
      }
      case 'task': {
        return args.description || '(no description)';
      }
      case 'todowrite': {
        const todos = args.todos;
        if (Array.isArray(todos)) {
          return `${todos.length} todo(s)`;
        }
        return '(todos)';
      }
      default:
        return entry.arguments.length > 60
          ? entry.arguments.slice(0, 60) + '...'
          : entry.arguments;
    }
  } catch {
    return entry.arguments || '...';
  }
}

function getResultSummary(entry: ToolCallEntry): string {
  if (!entry.result) return ' ...';
  const output = entry.result.output;
  if (entry.result.isError) return ' failed';

  const name = entry.name;
  const canonical = ['read', 'list', 'grep', 'glob', 'skill'].includes(name) ? name : '';

  switch (canonical) {
    case 'read': {
      return output.length > 80
        ? ` ${output.split('\n')[0].slice(0, 60)}...`
        : ` ${output.split('\n')[0] || ''}`;
    }
    case 'list': {
      const count = output.split('\n').filter((l) => l.trim()).length;
      return ` ${count} item(s)`;
    }
    case 'grep':
    case 'glob': {
      const count = output.split('\n').filter((l) => l.trim()).length;
      return ` ${count} match(es)`;
    }
    default: {
      const firstLine = output.split('\n')[0] || '';
      if (firstLine.length > 80) return ` ${firstLine.slice(0, 80)}...`;
      return firstLine ? ` ${firstLine}` : ' (empty)';
    }
  }
}

function getActionLabel(name: string): string {
  switch (name) {
    case 'read': return 'Read';
    case 'list': return 'List';
    case 'grep': return 'Search';
    case 'glob': return 'Find';
    case 'skill': return 'Loaded skill';
    default: return name;
  }
}

function getCollapsedLabel(entry: ToolCallEntry): string {
  const name = entry.name;
  if (isReadOnlyTool(name)) {
    return `${getActionLabel(name)} ${summarizeArguments(name, entry)}${entry.result ? getResultSummary(entry) : ' ...'}`;
  }
  if (isWriteTool(name)) {
    return `${name === 'apply_patch' ? 'Apply patch' : name === 'edit' ? 'Edit' : 'Write'} ${summarizeArguments(name, entry)}`;
  }
  if (isBash(name)) {
    return `bash ${summarizeArguments(name, entry)}`;
  }
  return `${name} ${summarizeArguments(name, entry)}`;
}

function getExitCode(entry: ToolCallEntry): number | null {
  return entry.result?.exitCode ?? null;
}

export function ToolCallRow({ entry }: Props) {
  const [expanded, setExpanded] = useState(false);

  // If the only result is empty or "Done", keep collapsed
  const isEmptyResult =
    entry.result &&
    entry.resultComplete &&
    (!entry.result.output || entry.result.output.trim() === '' || entry.result.output.trim() === 'Done');

  // For bash: auto-expand if error or has output
  const shouldAutoExpand =
    isBash(entry.name) && entry.resultComplete && !isEmptyResult;
  // For write tools: auto-expand to show diff
  const hasDiff = entry.result && entry.result.diff;

  const isExpanded = expanded || shouldAutoExpand || (hasDiff && entry.resultComplete);

  return (
    <div className={`my-2 overflow-hidden rounded-lg border ${getToolBg(entry.name)}`}>
      {/* Collapsed Header (always visible) */}
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-black/5 dark:hover:bg-white/5"
      >
        <svg className={`h-4 w-4 flex-shrink-0 ${getToolColor(entry.name)}`} fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d={getToolIcon(entry.name)} />
        </svg>
        <span className="flex-1 truncate text-xs font-medium text-neutral-700 dark:text-neutral-300">
          {getCollapsedLabel(entry)}
        </span>

        {/* Spinner for incomplete tool calls */}
        {!entry.resultComplete && entry.argumentsComplete && (
          <Loader2 className="h-3.5 w-3.5 animate-spin text-neutral-400" />
        )}

        {/* Expand/collapse indicator */}
        {entry.resultComplete && (
          <ChevronDown
            className={`h-3.5 w-3.5 text-neutral-400 transition-transform ${isExpanded ? 'rotate-180' : ''}`}
          />
        )}
      </button>

      {/* Expanded Content */}
      {isExpanded && entry.result && (
        <div className="border-t border-inherit">
          <div className="px-3 py-2">
            {/* Diff display for write/edit */}
            {isWriteTool(entry.name) && entry.result.diff && (
              <DiffRenderer diff={entry.result.diff} filepath={entry.result.filepath || ''} />
            )}

            {/* Bash output */}
            {isBash(entry.name) && (
              <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-xs leading-relaxed text-neutral-700 dark:text-neutral-300">
                {entry.result.output}
              </pre>
            )}

            {/* Read-only tools: render as markdown */}
            {isReadOnlyTool(entry.name) && (
              <MarkdownRenderer content={entry.result.output} />
            )}

            {/* Default: render as markdown */}
            {!isReadOnlyTool(entry.name) && !isBash(entry.name) && !isWriteTool(entry.name) && (
              <MarkdownRenderer content={entry.result.output} />
            )}
          </div>

          {/* Exit code for bash commands */}
          {isBash(entry.name) && getExitCode(entry) !== null && (
            <div className="border-t border-inherit bg-black/5 px-3 py-1 text-xs dark:bg-white/5">
              <span className="text-neutral-500 dark:text-neutral-400">
                Exit code: {getExitCode(entry)}
                {getExitCode(entry) === 0 ? (
                  <span className="text-green-600 dark:text-green-400"> &#10003;</span>
                ) : (
                  <span className="text-red-600 dark:text-red-400"> &#10007;</span>
                )}
              </span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
