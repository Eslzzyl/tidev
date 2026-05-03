import { useState, useRef, useEffect } from 'react';
import {
  Loader2,
  ChevronDown,
  FileText,
  FolderTree,
  Search,
  Files,
  FileEdit,
  FileDiff,
  Terminal,
  Sparkles,
  LayoutTemplate,
  ListTodo,
  Wrench,
} from 'lucide-react';
import type { ToolCallEntry } from '../../types/round';
import { MarkdownRenderer } from './MarkdownRenderer';
import { DiffRenderer } from './DiffRenderer';
import { CodeLinesRenderer } from './CodeLinesRenderer';
import { TodoRenderer } from './TodoRenderer';

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

function isTodo(name: string): boolean {
  return name === 'todowrite';
}

function getToolIcon(name: string): React.ComponentType<{ className?: string }> {
  switch (name) {
    case 'read':
      return FileText;
    case 'list':
      return FolderTree;
    case 'grep':
      return Search;
    case 'glob':
      return Files;
    case 'write':
    case 'edit':
      return FileEdit;
    case 'apply_patch':
      return FileDiff;
    case 'bash':
      return Terminal;
    case 'skill':
      return Sparkles;
    case 'task':
      return LayoutTemplate;
    case 'todowrite':
      return ListTodo;
    default:
      return Wrench;
  }
}

function getToolColor(name: string): string {
  if (isReadOnlyTool(name)) return 'text-blue-600 dark:text-blue-400';
  if (isWriteTool(name)) return 'text-emerald-600 dark:text-emerald-400';
  if (isBash(name)) return 'text-violet-600 dark:text-violet-400';
  if (name === 'task') return 'text-amber-600 dark:text-amber-400';
  if (isTodo(name)) return 'text-rose-600 dark:text-rose-400';
  return 'text-neutral-600 dark:text-neutral-400';
}

function getToolBg(name: string): string {
  if (isReadOnlyTool(name)) return 'bg-blue-50 dark:bg-blue-950/30 border-blue-200 dark:border-blue-800';
  if (isWriteTool(name)) return 'border-neutral-200 dark:border-neutral-700';
  if (isBash(name)) return 'bg-violet-50 dark:bg-violet-950/30 border-violet-200 dark:border-violet-800';
  if (name === 'task') return 'bg-amber-50 dark:bg-amber-950/30 border-amber-200 dark:border-amber-800';
  if (isTodo(name)) return 'bg-rose-50 dark:bg-rose-950/30 border-rose-200 dark:border-rose-800';
  return 'bg-neutral-50 dark:bg-neutral-800 border-neutral-200 dark:border-neutral-700';
}

function getToolLabel(name: string): string {
  switch (name) {
    case 'read': return 'Read';
    case 'list': return 'List';
    case 'grep': return 'Search';
    case 'glob': return 'Find';
    case 'skill': return 'Loaded skill';
    case 'bash': return 'bash';
    case 'todowrite': return 'Todos';
    default: return name;
  }
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
          return `${todos.length} item(s)`;
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
  const canonical = ['list', 'grep', 'glob', 'skill'].includes(name) ? name : '';

  switch (canonical) {
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

function getCollapsedLabel(entry: ToolCallEntry): string {
  const name = entry.name;
  if (isReadOnlyTool(name)) {
    return `${getToolLabel(name)} ${summarizeArguments(name, entry)}${entry.result ? getResultSummary(entry) : ' ...'}`;
  }
  if (isWriteTool(name)) {
    return `${name === 'apply_patch' ? 'Apply patch' : name === 'edit' ? 'Edit' : 'Write'} ${summarizeArguments(name, entry)}`;
  }
  if (isTodo(name)) {
    return `${getToolLabel(name)} ${summarizeArguments(name, entry)}`;
  }
  return `${name} ${summarizeArguments(name, entry)}`;
}

function getExitCode(entry: ToolCallEntry): number | null {
  return entry.result?.exitCode ?? null;
}

/**
 * Get the bash command from parsed arguments, for display purposes.
 */
function getBashCommand(entry: ToolCallEntry): string {
  try {
    const args = JSON.parse(entry.arguments);
    return args.command || '';
  } catch {
    return '';
  }
}

export function ToolCallRow({ entry }: Props) {
  const [expanded, setExpanded] = useState(false);
  const didAutoExpand = useRef(false);

  // Determine auto-expand conditions
  const isEmptyResult =
    entry.result &&
    entry.resultComplete &&
    (!entry.result.output || entry.result.output.trim() === '' || entry.result.output.trim() === 'Done');

  const hasBashOutput = isBash(entry.name) && entry.resultComplete && !isEmptyResult;
  const hasDiff = entry.result && entry.result.diff;

  // Auto-expand once when result arrives (like defaultExpanded for ThinkingBlock)
  // After that, user toggle fully controls the state
  useEffect(() => {
    if (didAutoExpand.current) return;
    if (hasBashOutput || (hasDiff && entry.resultComplete)) {
      didAutoExpand.current = true;
      setExpanded(true);
    }
  }, [hasBashOutput, hasDiff, entry.resultComplete]);

  // isExpanded is purely controlled by the `expanded` state — user toggle always works
  const isExpanded = expanded;

  function handleToggle() {
    setExpanded((prev) => !prev);
  }

  return (
    <div className={`my-2 overflow-hidden rounded-lg border ${getToolBg(entry.name)}`}>
      {/* Collapsed Header (always visible) */}
      <button
        onClick={handleToggle}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-black/5 dark:hover:bg-white/5"
      >
        {(() => {
          const Icon = getToolIcon(entry.name);
          return <Icon className={`h-3.5 w-3.5 flex-shrink-0 ${getToolColor(entry.name)}`} />;
        })()}

        {/* Collapsed label: show different layout for bash */}
        {isBash(entry.name) ? (
          <div className="flex flex-1 flex-col min-w-0">
            <span className="text-xs font-medium text-neutral-700 dark:text-neutral-300">
              bash
            </span>
            <span className="truncate font-mono text-xs text-neutral-500 dark:text-neutral-400">
              $ {getBashCommand(entry) || '...'}
            </span>
          </div>
        ) : (
          <span className="flex-1 truncate text-xs font-medium text-neutral-700 dark:text-neutral-300">
            {getCollapsedLabel(entry)}
          </span>
        )}

        {/* Spinner for incomplete tool calls */}
        {!entry.resultComplete && entry.argumentsComplete && (
          <Loader2 className="h-3.5 w-3.5 animate-spin text-neutral-400" />
        )}

        {/* Expand/collapse indicator */}
        {entry.resultComplete && (
          <ChevronDown
            className={`h-3.5 w-3.5 flex-shrink-0 text-neutral-400 transition-transform ${isExpanded ? 'rotate-180' : ''}`}
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

            {/* Bash: show command + output */}
            {isBash(entry.name) && (
              <div className="space-y-1">
                <div className="overflow-x-auto whitespace-pre-wrap break-all rounded bg-black/5 px-3 py-1.5 font-mono text-xs leading-relaxed text-neutral-600 dark:bg-white/5 dark:text-neutral-400">
                  $ {getBashCommand(entry)}
                </div>
                {entry.result.output && (
                  <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-xs leading-relaxed text-neutral-700 dark:text-neutral-300">
                    {entry.result.output}
                  </pre>
                )}
              </div>
            )}

            {/* Read tool: render as code lines with line numbers */}
            {entry.name === 'read' && (
              <CodeLinesRenderer output={entry.result.output} filepath={entry.result.filepath} />
            )}

            {/* Todo tool: render as structured list */}
            {isTodo(entry.name) && (
              <TodoRenderer output={entry.result.output} />
            )}

            {/* Other read-only tools: render as markdown */}
            {isReadOnlyTool(entry.name) && entry.name !== 'read' && (
              <MarkdownRenderer content={entry.result.output} />
            )}

            {/* Default: render as markdown */}
            {!isReadOnlyTool(entry.name) && !isBash(entry.name) && !isWriteTool(entry.name) && !isTodo(entry.name) && (
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
