import { useMemo } from 'react';
import { X, CheckCircle2, Circle, XCircle, AlertTriangle } from 'lucide-react';
import { useSessionStore } from '../../stores/useSessionStore';
import { useUIStore } from '../../stores/useUIStore';
import type { FileDiff, TodoItem, TokenUsage } from '../../types/api';
import { formatNumber, formatToken, formatWorkspace } from '../../utils/format';

export function RightSidebar() {
  const messages = useSessionStore((s) => s.messages);
  const currentSession = useSessionStore((s) => s.currentSession);

  const closeMobileRightSidebar = useUIStore((s) => s.closeMobileRightSidebar);

  // Stats derived from messages
  const stats = useMemo(() => {
    const assistantMessages = messages.filter((m) => m.role === 'assistant');
    let totalTokens = 0;
    let inputTokens = 0;
    let outputTokens = 0;

    for (const msg of assistantMessages) {
      const usage = msg.token_usage as TokenUsage | undefined;
      if (usage) {
        totalTokens += usage.total_tokens || 0;
        inputTokens += usage.input_tokens || 0;
        outputTokens += usage.output_tokens || 0;
      }
    }

    return {
      requestCount: assistantMessages.length,
      totalTokens,
      inputTokens,
      outputTokens,
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

    for (const msg of messages) {
      const fileDiffsArr = msg.file_diffs as FileDiff[] | undefined;
      if (fileDiffsArr && Array.isArray(fileDiffsArr)) {
        for (const diff of fileDiffsArr) {
          diffs.push({
            path: diff.path || diff.file_path || 'unknown',
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

  // Todos
  const todos = useMemo(() => {
    const items: Array<{
      content: string;
      status: 'pending' | 'in_progress' | 'completed' | 'cancelled';
      priority: 'low' | 'medium' | 'high';
    }> = [];

    for (const msg of messages) {
      const todosArr = msg.todos as TodoItem[] | undefined;
      if (todosArr && Array.isArray(todosArr)) {
        for (const todo of todosArr) {
          items.push({
            content: todo.content || todo.title || 'Untitled',
            status: todo.status || 'pending',
            priority: todo.priority || 'medium',
          });
        }
      }
    }

    return items;
  }, [messages]);

  // Undo state
  const isUndoActive = useMemo(() => {
    const summary = currentSession?.context_summary;
    return summary?.includes('revert') || false;
  }, [currentSession]);

  function getFileStatusIcon(status: string): string {
    switch (status) {
      case 'added': return '+';
      case 'deleted': return '-';
      case 'modified': default: return '~';
    }
  }

  function getStatusColorClass(status: string): string {
    switch (status) {
      case 'added': return 'text-green-600 dark:text-green-400';
      case 'deleted': return 'text-red-600 dark:text-red-400';
      case 'modified': default: return 'text-amber-600 dark:text-amber-400';
    }
  }

  function getTodoIcon(status: string): React.ReactNode {
    switch (status) {
      case 'completed':
        return <CheckCircle2 className="h-3.5 w-3.5" />;
      case 'in_progress':
        return <Circle className="h-3.5 w-3.5 fill-current" />;
      case 'cancelled':
        return <XCircle className="h-3.5 w-3.5" />;
      case 'pending':
      default:
        return <Circle className="h-3.5 w-3.5" />;
    }
  }

  return (
    <div className="flex h-full flex-col bg-neutral-50 dark:bg-neutral-900">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
        <h2 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">Info</h2>
        <button
          onClick={closeMobileRightSidebar}
          className="rounded p-1 text-neutral-500 hover:bg-neutral-100 md:hidden dark:text-neutral-400 dark:hover:bg-neutral-800"
          aria-label="Close info panel"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {currentSession ? (
          <>
            {/* Session Info */}
            <div className="space-y-2">
              <h3 className="text-xs font-semibold uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
                Session
              </h3>
              <div className="space-y-1.5 text-xs text-neutral-700 dark:text-neutral-300">
                <div className="flex justify-between">
                  <span className="text-neutral-500 dark:text-neutral-400">Model</span>
                  <span className="font-medium">{currentSession.model_display_name}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-neutral-500 dark:text-neutral-400">Provider</span>
                  <span className="font-medium">{currentSession.provider_display_name}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-neutral-500 dark:text-neutral-400">Workspace</span>
                  <span className="max-w-[140px] truncate font-medium" title={currentSession.workspace_root}>
                    {formatWorkspace(currentSession.workspace_root)}
                  </span>
                </div>
              </div>
            </div>

            {/* Token Usage */}
            <div className="space-y-2">
              <h3 className="text-xs font-semibold uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
                Usage
              </h3>
              <div className="space-y-1.5 text-xs text-neutral-700 dark:text-neutral-300">
                <div className="flex justify-between">
                  <span className="text-neutral-500 dark:text-neutral-400">Requests</span>
                  <span>{formatNumber(stats.requestCount)}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-neutral-500 dark:text-neutral-400">Total tokens</span>
                  <span>{formatToken(stats.totalTokens)}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-neutral-500 dark:text-neutral-400">Input tokens</span>
                  <span>{formatToken(stats.inputTokens)}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-neutral-500 dark:text-neutral-400">Output tokens</span>
                  <span>{formatToken(stats.outputTokens)}</span>
                </div>
              </div>
            </div>

            {/* File Diffs */}
            {fileDiffs.length > 0 && (
              <div className="space-y-2">
                <h3 className="text-xs font-semibold uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
                  Files Changed
                </h3>
                <div className="space-y-1">
                  {fileDiffs.map((fd, idx) => (
                    <div key={idx} className="flex items-center gap-2 rounded bg-white px-2 py-1.5 text-xs dark:bg-neutral-800">
                      <span className={`font-mono text-sm font-bold ${getStatusColorClass(fd.status)}`}>
                        {getFileStatusIcon(fd.status)}
                      </span>
                      <span className="flex-1 truncate text-neutral-700 dark:text-neutral-300">
                        {fd.path}
                      </span>
                      {fd.additions > 0 && (
                        <span className="text-green-600 dark:text-green-400">+{fd.additions}</span>
                      )}
                      {fd.deletions > 0 && (
                        <span className="text-red-600 dark:text-red-400">-{fd.deletions}</span>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Todos */}
            {todos.length > 0 && (
              <div className="space-y-2">
                <h3 className="text-xs font-semibold uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
                  Todos
                </h3>
                <ul className="space-y-1">
                  {todos.map((todo, idx) => (
                    <li key={idx} className="flex items-start gap-2 text-xs">
                      <span className={`mt-0.5 flex-shrink-0 ${
                        todo.status === 'completed'
                          ? 'text-green-600 dark:text-green-400'
                          : todo.status === 'in_progress'
                            ? 'text-blue-600 dark:text-blue-400'
                            : 'text-neutral-400 dark:text-neutral-500'
                      }`}>
                        {getTodoIcon(todo.status)}
                      </span>
                      <span className={`text-neutral-700 dark:text-neutral-300 ${
                        todo.status === 'completed' ? 'line-through opacity-50' : ''
                      }`}>
                        {todo.content}
                        {todo.priority === 'high' && (
                          <AlertTriangle className="ml-1 inline h-3 w-3 text-amber-500" />
                        )}
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            )}

            {/* Undo State */}
            {isUndoActive && (
              <div className="rounded bg-amber-50 p-3 dark:bg-amber-950">
                <p className="flex items-center gap-2 text-xs text-amber-800 dark:text-amber-200">
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
