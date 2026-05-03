import { useState, useMemo, useEffect, useRef } from "react";
import { X, Search, Forklift, Undo2, User, Bot } from "lucide-react";
import type { Message } from "../../types/api";
import { formatTime } from "../../utils/format";

interface MessageDialogProps {
  isOpen: boolean;
  messages: Message[];
  onClose: () => void;
  onFork: (messageId: string) => void;
  onUndo: (messageId: string) => void;
  isUndoing: boolean;
  isForking: boolean;
}

export function MessageDialog({
  isOpen,
  messages,
  onClose,
  onFork,
  onUndo,
  isUndoing,
  isForking,
}: MessageDialogProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // Focus input when dialog opens
  useEffect(() => {
    if (isOpen) {
      const raf = requestAnimationFrame(() => {
        inputRef.current?.focus();
      });
      return () => cancelAnimationFrame(raf);
    }
  }, [isOpen]);

  // Filter user messages by search query (matching TUI behavior)
  const filteredUserMessageIds = useMemo(() => {
    if (!searchQuery.trim()) return null; // null means show all
    const query = searchQuery.trim().toLowerCase();
    const ids = new Set(
      messages
        .filter(
          (m) =>
            m.role === "user" &&
            m.content.toLowerCase().includes(query),
        )
        .map((m) => m.id),
    );
    return ids;
  }, [messages, searchQuery]);

  // Determine if a message should be visible
  const isMessageVisible = (msg: Message) => {
    if (!filteredUserMessageIds) return true;
    // User messages must match the filter
    if (msg.role === "user") return filteredUserMessageIds.has(msg.id);
    // Assistant/system messages between visible user messages are shown
    return true;
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div className="flex h-[80vh] w-full max-w-2xl flex-col rounded-lg bg-white shadow-lg dark:bg-neutral-900">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
          <h3 className="text-lg font-semibold text-neutral-900 dark:text-neutral-100">
            Session Messages
          </h3>
          <button
            onClick={onClose}
            className="rounded p-1 text-neutral-400 hover:bg-neutral-100 hover:text-neutral-600 dark:hover:bg-neutral-800 dark:hover:text-neutral-300"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Search input */}
        <div className="border-b border-neutral-200 px-4 py-2 dark:border-neutral-800">
          <div className="relative">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-neutral-400" />
            <input
              ref={inputRef}
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search user messages..."
              className="w-full rounded-lg border border-neutral-300 bg-white py-2 pl-9 pr-3 text-sm text-neutral-900 placeholder-neutral-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100 dark:placeholder-neutral-500 dark:focus:border-blue-400"
            />
          </div>
        </div>

        {/* Message list */}
        <div className="flex-1 overflow-y-auto">
          {messages.length === 0 ? (
            <div className="flex items-center justify-center py-12 text-sm text-neutral-400">
              No messages in this session.
            </div>
          ) : (
            <div className="divide-y divide-neutral-100 dark:divide-neutral-800">
              {messages.map((msg) => {
                const isVisible = isMessageVisible(msg);
                const isUser = msg.role === "user";
                const isAssistant = msg.role === "assistant";

                if (!isVisible) return null;

                return (
                  <div
                    key={msg.id}
                    className={`px-4 py-3 transition-colors hover:bg-neutral-50 dark:hover:bg-neutral-800/50 ${
                      !isUser ? "opacity-70" : ""
                    }`}
                  >
                    <div className="flex items-start gap-3">
                      {/* Role icon */}
                      <div className="flex-shrink-0 pt-0.5">
                        {isUser ? (
                          <div className="flex h-7 w-7 items-center justify-center rounded-full bg-neutral-200 text-xs font-medium text-neutral-700 dark:bg-neutral-700 dark:text-neutral-300">
                            <User className="h-3.5 w-3.5" />
                          </div>
                        ) : (
                          <div className="flex h-7 w-7 items-center justify-center rounded-full bg-blue-100 text-xs font-medium text-blue-700 dark:bg-blue-900/50 dark:text-blue-300">
                            <Bot className="h-3.5 w-3.5" />
                          </div>
                        )}
                      </div>

                      {/* Message content */}
                      <div className="min-w-0 flex-1">
                        <div className="mb-1 flex items-center gap-2">
                          <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400">
                            {isUser ? "You" : isAssistant ? "Assistant" : msg.role}
                          </span>
                          <span className="text-xs text-neutral-400 dark:text-neutral-600">
                            {formatTime(msg.created_at)}
                          </span>
                        </div>
                        <p className="line-clamp-2 text-sm leading-relaxed text-neutral-700 dark:text-neutral-300">
                          {msg.content || "(empty)"}
                        </p>
                      </div>

                      {/* Action buttons (only for user messages) */}
                      {isUser && (
                        <div className="flex flex-shrink-0 items-center gap-1.5">
                          <button
                            onClick={() => onFork(msg.id)}
                            disabled={isForking}
                            className="rounded-md px-2.5 py-1.5 text-xs font-medium text-blue-600 transition-colors hover:bg-blue-50 disabled:opacity-50 dark:text-blue-400 dark:hover:bg-blue-900/30"
                            title="Fork session from this message"
                          >
                            <Forklift className="mr-1 inline h-3.5 w-3.5" />
                            Fork
                          </button>
                          <button
                            onClick={() => onUndo(msg.id)}
                            disabled={isUndoing}
                            className="rounded-md px-2.5 py-1.5 text-xs font-medium text-amber-600 transition-colors hover:bg-amber-50 disabled:opacity-50 dark:text-amber-400 dark:hover:bg-amber-900/30"
                            title="Undo to this message"
                          >
                            <Undo2 className="mr-1 inline h-3.5 w-3.5" />
                            Undo
                          </button>
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="border-t border-neutral-200 px-4 py-2 text-xs text-neutral-400 dark:border-neutral-800">
          {messages.filter((m) => m.role === "user").length} user messages ·{" "}
          {messages.length} total messages
        </div>
      </div>
    </div>
  );
}
