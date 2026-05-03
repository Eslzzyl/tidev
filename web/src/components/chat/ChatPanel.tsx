import { useMemo, useEffect, useState, useCallback } from 'react';
import { Menu, Settings, Info } from 'lucide-react';
import { useSessionStore } from '../../stores/useSessionStore';
import { useUIStore } from '../../stores/useUIStore';
import { useSSE } from '../../hooks/useSSE';
import { buildRounds } from '../../utils/round';
import { MessageRound } from './MessageRound';
import { MessageInput } from './MessageInput';

export function ChatPanel() {
  const messages = useSessionStore((s) => s.messages);
  const currentSessionId = useSessionStore((s) => s.currentSessionId);
  const currentSession = useSessionStore((s) => s.currentSession);
  const isDraftSession = useSessionStore((s) => s.isDraftSession);
  const draftTitle = useSessionStore((s) => s.draftTitle);

  const toggleMobileMenu = useUIStore((s) => s.toggleMobileMenu);
  const toggleRightSidebar = useUIStore((s) => s.toggleRightSidebar);
  const toggleMobileRightSidebar = useUIStore((s) => s.toggleMobileRightSidebar);
  const toggleSettings = useUIStore((s) => s.toggleSettings);

  const streamingRound = useSSE(currentSessionId);

  const completedRounds = useMemo(() => buildRounds(messages), [messages]);

  const allRounds = useMemo(() => {
    const rounds = [...completedRounds];
    if (streamingRound) {
      rounds.push(streamingRound);
    }
    return rounds;
  }, [completedRounds, streamingRound]);

  // Auto-scroll
  const [messagesContainerRef, setMessagesContainerRef] = useState<HTMLDivElement | null>(null);
  const [messagesEndRef, setMessagesEndRef] = useState<HTMLDivElement | null>(null);
  const [shouldAutoScroll, setShouldAutoScroll] = useState(true);
  const [isFirstLoad, setIsFirstLoad] = useState(true);

  useEffect(() => {
    if (isFirstLoad && allRounds.length > 0 && messagesEndRef) {
      messagesEndRef.scrollIntoView({ behavior: 'instant' });
      setIsFirstLoad(false);
    }
  }, [allRounds.length, isFirstLoad, messagesEndRef]);

  useEffect(() => {
    if (!isFirstLoad && shouldAutoScroll && messagesEndRef) {
      messagesEndRef.scrollIntoView({ behavior: 'smooth' });
    }
  });

  useEffect(() => {
    if (currentSessionId) {
      setIsFirstLoad(true);
    }
  }, [currentSessionId]);

  const handleScroll = useCallback(() => {
    if (!messagesContainerRef) return;
    const { scrollHeight, scrollTop, clientHeight } = messagesContainerRef;
    const isNearBottom = scrollHeight - scrollTop - clientHeight < 100;
    setShouldAutoScroll(isNearBottom);
  }, [messagesContainerRef]);

  const scrollToBottom = useCallback(() => {
    if (messagesEndRef) {
      messagesEndRef.scrollIntoView({ behavior: 'smooth' });
      setShouldAutoScroll(true);
    }
  }, [messagesEndRef]);

  return (
    <div className="flex h-full flex-col bg-white dark:bg-neutral-950">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
        <div className="flex items-center gap-3">
          <button
            onClick={toggleMobileMenu}
            className="rounded p-1 text-neutral-600 hover:bg-neutral-100 md:hidden dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Open menu"
          >
            <Menu className="h-5 w-5" />
          </button>

          {isDraftSession ? (
            <div>
              <h1 className="text-sm font-semibold text-blue-600 dark:text-blue-400">{draftTitle}</h1>
              <p className="text-xs text-neutral-500 dark:text-neutral-400">Draft Session</p>
            </div>
          ) : currentSession ? (
            <div>
              <h1 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">{currentSession.title}</h1>
              <p className="text-xs text-neutral-500 dark:text-neutral-400">{currentSession.model_display_name}</p>
            </div>
          ) : (
            <h1 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">Select a session</h1>
          )}
        </div>

        <div className="flex items-center gap-2">
          {/* Settings button */}
          <button
            onClick={toggleSettings}
            className="rounded p-2 text-neutral-600 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Settings"
          >
            <Settings className="h-5 w-5" />
          </button>

          {/* Right sidebar toggle (desktop) */}
          <button
            onClick={toggleRightSidebar}
            className="hidden rounded p-2 text-neutral-600 hover:bg-neutral-100 md:block dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Toggle info panel"
          >
            <Info className="h-5 w-5" />
          </button>

          {/* Mobile right sidebar toggle */}
          <button
            onClick={toggleMobileRightSidebar}
            className="rounded p-2 text-neutral-600 hover:bg-neutral-100 md:hidden dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Open info panel"
          >
            <Info className="h-5 w-5" />
          </button>
        </div>
      </div>

      {/* Messages Area */}
      <div
        ref={setMessagesContainerRef}
        onScroll={handleScroll}
        className="flex-1 overflow-y-auto"
      >
        {!currentSessionId && !isDraftSession ? (
          <div className="flex h-full items-center justify-center">
            <div className="text-center">
              <p className="text-neutral-500 dark:text-neutral-400">
                Select a session or create a new one to start chatting
              </p>
            </div>
          </div>
        ) : allRounds.length === 0 && !streamingRound ? (
          <div className="flex h-full items-center justify-center">
            <div className="text-center">
              <p className="text-neutral-500 dark:text-neutral-400">
                {isDraftSession
                  ? 'Type your first message to create the session'
                  : 'No messages yet. Start a conversation!'}
              </p>
            </div>
          </div>
        ) : (
          <div className="divide-y divide-neutral-100 dark:divide-neutral-900">
            {allRounds.map((round) => (
              <MessageRound key={round.id} round={round} />
            ))}
          </div>
        )}
        <div ref={setMessagesEndRef} />
      </div>

      {/* Input Area */}
      <MessageInput />
    </div>
  );
}
