import { useMemo } from 'react';
import { X, CheckCircle2, Circle, Clock, XCircle, AlertTriangle, Plus, Minus, Pencil } from 'lucide-react';
import { useSessionStore } from '../../stores/useSessionStore';
import { useUIStore } from '../../stores/useUIStore';
import type { FileDiff, TodoItem, TokenUsage } from '../../types/api';
import { formatNumber, formatToken, formatWorkspace } from '../../utils/format';

export function RightSidebar() {
  const messages = useSessionStore((s) => s.messages);
  const currentSession = useSessionStore((s) => s.currentSession);
  const sessionTodos = useSessionStore((s) => s.todos);

  const closeMobileRightSidebar = useUIStore((s) => s.closeMobileRightSidebar);

  // Stats derived from messages
  const stats = useMemo(() => {
    const assistantMessages = messages.filter((m) => m.role === 'assistant');
    let totalTokens = 0;
    let inputTokens = 0;
    let outputTokens = 0;
    let cacheReadTokens = 0;
    let cacheWriteTokens = 0;
    let totalTpsSum = 0;
    let tpsCount = 0;

    for (const msg of assistantMessages) {
      const usage = msg.token_usage as TokenUsage | undefined;
      if (usage) {
        totalTokens += usage.total_tokens || 0;
        inputTokens += usage.input_tokens || 0;
        outputTokens += usage.output_tokens || 0;
        cacheReadTokens += usage.cache_read_tokens || 0;
        cacheWriteTokens += usage.cache_write_tokens || 0;
      }
      if (msg.tokens_per_second != null) {
        totalTpsSum += msg.tokens_per_second;
        tpsCount += 1;
      }
    }

    const avgTps = tpsCount > 0 ? totalTpsSum / tpsCount : null;

    return {
      requestCount: assistantMessages.length,
      totalTokens,
      inputTokens,
      outputTokens,
      cacheReadTokens,
      cacheWriteTokens,
      avgTps,
    };
  }, [messages]);

  // File diffs
  const fileDiffs = useMemo(() => {
    const diffs: Array<{
      path: string;
      status: 'added' | 'modified' | 'deleted';
      additions: number;
      deletions: number;
    }> = [];
    const seen = new Set<string>();

    for (const msg of messages) {
      const fileDiffsArr = msg.file_diffs as FileDiff[] | undefined;
      if (fileDiffsArr && Array.isArray(fileDiffsArr)) {
        for (const diff of fileDiffsArr) {
          const key = diff.path || diff.file_path || 'unknown';
          if (seen.has(key)) continue;
          seen.add(key);
          diffs.push({
            path: key,
            status: diff.status,
            additions: diff.additions,
            deletions: diff.deletions,
          });
        }
      }
    }

    const statusOrder = { modified: 0, added: 1, deleted: 2 };
    return diffs.sort((a, b) => statusOrder[a.status] - statusOrder[b.status]);
  }, [messages]);

  // Session-level todos from store (aggregated by backend)
  const todos: TodoItem[] = sessionTodos;

  // Undo state detection
  const isUndoActive = useMemo(() => {
    return messages.some((m) => m.content.includes('undo') || m.role === 'system');
  }, [messages]);

  const workspacePath = currentSession?.workspace_root ?? '';
  const displayPath = formatWorkspace(workspacePath);

  const providerName = currentSession?.provider_display_name ?? '';
  const modelName = currentSession?.model_display_name ?? '';

  // Todo status icon matching TUI style
  const getTodoIcon = (status: string) => {
    switch (status) {
      case 'completed':
        return <CheckCircle2 className="h-4 w-4 text-green-500 dark:text-green-400" />;
      case 'in_progress':
        return <Clock className="h-4 w-4 text-blue-500 dark:text-blue-400" />;
      case 'cancelled':
        return <XCircle className="h-4 w-4 text-neutral-400 dark:text-neutral-500" />;
      default:
        return <Circle className="h-4 w-4 text-neutral-500 dark:text-neutral-400" />;
    }
  };

  // File status icon matching TUI style
  const getFileStatusIcon = (status: string) => {
    switch (status) {
      case 'added':
        return <Plus className="h-4 w-4 text-green-600 dark:text-green-400" />;
      case 'deleted':
        return <Minus className="h-4 w-4 text-red-600 dark:text-red-400" />;
      default:
        return <Pencil className="h-4 w-4 text-amber-600 dark:text-amber-400" />;
    }
  };

  const getFileStatusColor = (status: string) => {
    switch (status) {
      case 'added':
        return 'text-green-700 dark:text-green-400';
      case 'deleted':
        return 'text-red-700 dark:text-red-400';
      default:
        return 'text-amber-700 dark:text-amber-400';
    }
  };

  return (
    <div className="flex h-full flex-col bg-white dark:bg-neutral-950">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
          Info
        </h2>
        <button
          onClick={closeMobileRightSidebar}
          className="rounded p-1 text-neutral-500 hover:bg-neutral-100 md:hidden dark:text-neutral-400 dark:hover:bg-neutral-800"
          aria-label="Close info panel"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-4 py-3">
        {currentSession || messages.length > 0 ? (
          <>
            {/* Workspace */}
            {workspacePath && (
              <div className="mb-4">
                <h3 className="mb-1 text-xs font-bold uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
                  Workspace
                </h3>
                <p className="truncate text-sm text-neutral-600 dark:text-neutral-400" title={workspacePath}>
                  {displayPath}
                </p>
              </div>
            )}

            {/* Model */}
            {(modelName || providerName) && (
              <div className="mb-4">
                <h3 className="mb-1 text-xs font-bold uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
                  Model
                </h3>
                <p className="text-sm text-neutral-800 dark:text-neutral-200">
                  {modelName}
                </p>
                <p className="text-xs text-neutral-500 dark:text-neutral-400">
                  {providerName}
                </p>
                {stats.avgTps != null && (
                  <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
                    Speed: {stats.avgTps.toFixed(1)} t/s (avg)
                  </p>
                )}
              </div>
            )}

            {/* Tokens */}
            {(stats.totalTokens > 0 || stats.requestCount > 0) && (
              <>
                <div className="mb-4">
                  <h3 className="mb-1 text-xs font-bold uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
                    Tokens
                  </h3>
                  <div className="space-y-0.5 text-sm">
                    <p className="text-neutral-800 dark:text-neutral-200">
                      Total: {formatToken(stats.totalTokens)}
                    </p>
                    <p className="text-neutral-500 dark:text-neutral-400">
                      In: {formatToken(stats.inputTokens)}
                    </p>
                    <p className="text-neutral-500 dark:text-neutral-400">
                      Cache: {formatToken(stats.cacheReadTokens)}
                    </p>
                    <p className="text-neutral-500 dark:text-neutral-400">
                      Out: {formatToken(stats.outputTokens)}
                    </p>
                  </div>
                </div>

                {/* Request count */}
                <div className="mb-4">
                  <p className="text-sm text-neutral-800 dark:text-neutral-200">
                    Requests: {stats.requestCount}
                  </p>
                </div>
              </>
            )}

            {/* Changed Files */}
            <div className="mb-4">
              <h3 className="mb-1 text-xs font-bold uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
                Changed Files
              </h3>
              {fileDiffs.length === 0 ? (
                <p className="text-sm text-neutral-400 dark:text-neutral-500">
                  (no changes yet)
                </p>
              ) : (
                <ul className="space-y-1">
                  {fileDiffs.map((diff, idx) => (
                    <li key={idx} className="flex items-center gap-1.5 text-sm">
                      <span className="flex-shrink-0">
                        {getFileStatusIcon(diff.status)}
                      </span>
                      <span className={`flex-1 truncate ${getFileStatusColor(diff.status)}`}>
                        {diff.path.split('/').pop() || diff.path}
                      </span>
                      <span className={`flex-shrink-0 text-xs ${getFileStatusColor(diff.status)}`}>
                        +{diff.additions}/-{diff.deletions}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </div>

            {/* Todos */}
            <div className="mb-4">
              <h3 className="mb-1 text-xs font-bold uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
                Todos ({todos.length})
              </h3>
              {todos.length === 0 ? (
                <p className="text-sm text-neutral-400 dark:text-neutral-500">
                  (no todos yet)
                </p>
              ) : (
                <ul className="space-y-1.5">
                  {todos.map((todo, idx) => (
                    <li key={idx} className="flex items-start gap-1.5 text-sm">
                      <span className="mt-0.5 flex-shrink-0">
                        {getTodoIcon(todo.status)}
                      </span>
                      <span
                        className={`flex-1 ${
                          todo.status === 'completed' || todo.status === 'cancelled'
                            ? 'text-neutral-400 line-through dark:text-neutral-500'
                            : 'text-neutral-700 dark:text-neutral-300'
                        }`}
                      >
                        {todo.priority === 'high' && (
                          <AlertTriangle className="mr-0.5 inline h-3.5 w-3.5 text-amber-500" />
                        )}
                        {todo.content}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </div>

            {/* Undo State */}
            {isUndoActive && (
              <div className="rounded bg-amber-50 p-3 dark:bg-amber-950">
                <p className="flex items-center gap-2 text-sm text-amber-800 dark:text-amber-200">
                  <span>⚠</span>
                  <span>Undo active</span>
                </p>
                <p className="mt-1 text-xs text-amber-700 dark:text-amber-300">
                  Conversation was reverted. New messages will branch from this point.
                </p>
              </div>
            )}
          </>
        ) : (
          <p className="text-center text-sm text-neutral-500 dark:text-neutral-400">
            Select a session to view details
          </p>
        )}
      </div>
    </div>
  );
}
