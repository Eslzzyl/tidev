import { useMemo, useEffect, useState, useCallback } from 'react';
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
            <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
            </svg>
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
            <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
            </svg>
          </button>

          {/* Right sidebar toggle (desktop) */}
          <button
            onClick={toggleRightSidebar}
            className="hidden rounded p-2 text-neutral-600 hover:bg-neutral-100 md:block dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Toggle info panel"
          >
            <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
          </button>

          {/* Mobile right sidebar toggle */}
          <button
            onClick={toggleMobileRightSidebar}
            className="rounded p-2 text-neutral-600 hover:bg-neutral-100 md:hidden dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Open info panel"
          >
            <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
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
