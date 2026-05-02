import { useState, useEffect, useRef, useMemo, useCallback } from 'react';
import { useSessionStore } from '../../stores/useSessionStore';
import { useUIStore } from '../../stores/useUIStore';
import { api } from '../../api/client';
import type { ModelInfo } from '../../types/api';

type ThinkingOption = { label: string; value: string };

export function MessageInput() {
  const [inputValue, setInputValue] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Models state
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [modelDropdownOpen, setModelDropdownOpen] = useState(false);
  const [modelSearchQuery, setModelSearchQuery] = useState('');
  const [selectedModelId, setSelectedModelId] = useState<string | null>(null);
  const [selectedProviderId, setSelectedProviderId] = useState<string | null>(null);
  const [thinkingOptions, setThinkingOptions] = useState<ThinkingOption[]>([]);
  const [selectedThinking, setSelectedThinking] = useState<string>('');

  const dropdownRef = useRef<HTMLDivElement>(null);
  const thinkingDropdownRef = useRef<HTMLDivElement>(null);

  const currentSessionId = useSessionStore((s) => s.currentSessionId);
  const isDraftSession = useSessionStore((s) => s.isDraftSession);
  const mode = useSessionStore((s) => s.mode);
  const setMode = useSessionStore((s) => s.setMode);
  const toggleMode = useSessionStore((s) => s.toggleMode);
  const commitDraftSession = useSessionStore((s) => s.commitDraftSession);
  const setCurrentSessionId = useSessionStore((s) => s.setCurrentSessionId);
  const setCurrentRequestId = useSessionStore((s) => s.setCurrentRequestId);
  const currentRequestId = useSessionStore((s) => s.currentRequestId);
  const setMessages = useSessionStore((s) => s.setMessages);
  const isStreaming = useUIStore((s) => s.isStreaming);
  const setStreaming = useUIStore((s) => s.setStreaming);
  const setError = useSessionStore((s) => s.setError);

  const isInputEnabled = currentSessionId !== null || isDraftSession;

  // Filtered models
  const filteredModels = useMemo(() => {
    if (!modelSearchQuery.trim()) return models;
    const q = modelSearchQuery.toLowerCase();
    return models.filter(
      (m) =>
        m.display_name.toLowerCase().includes(q) ||
        m.id.toLowerCase().includes(q) ||
        m.provider_name.toLowerCase().includes(q)
    );
  }, [models, modelSearchQuery]);

  // Grouped models
  const groupedModels = useMemo(() => {
    const groups = new Map<string, ModelInfo[]>();
    for (const m of filteredModels) {
      const key = m.provider_name || m.provider_id;
      if (!groups.has(key)) {
        groups.set(key, []);
      }
      groups.get(key)!.push(m);
    }
    return groups;
  }, [filteredModels]);

  // Update thinking levels based on model
  const updateThinkingLevels = useCallback((modelId: string) => {
    const id = modelId.toLowerCase();
    if (id.includes('deepseek') && id.includes('4')) {
      setThinkingOptions([
        { label: 'Off', value: 'deepseek:Off' },
        { label: 'High', value: 'deepseek:High' },
        { label: 'Max', value: 'deepseek:Max' },
      ]);
      setSelectedThinking('deepseek:Off');
    } else if (id.includes('qwen') && id.includes('3.')) {
      setThinkingOptions([
        { label: 'Off', value: 'qwen:Off' },
        { label: 'On', value: 'qwen:On' },
      ]);
      setSelectedThinking('qwen:Off');
    } else if (id.includes('glm')) {
      setThinkingOptions([
        { label: 'Off', value: 'glm:Off' },
        { label: 'On', value: 'glm:On' },
      ]);
      setSelectedThinking('glm:Off');
    } else {
      setThinkingOptions([]);
      setSelectedThinking('');
    }
  }, []);

  // Load models
  useEffect(() => {
    api.listModels().then(({ models: modelList }) => {
      setModels(modelList);
      if (!selectedModelId && modelList.length > 0) {
        setSelectedModelId(modelList[0].id);
        setSelectedProviderId(modelList[0].provider_id);
        updateThinkingLevels(modelList[0].id);
      }
    }).catch(() => {});
  }, [selectedModelId, updateThinkingLevels]);

  // Close dropdowns on click outside
  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setModelDropdownOpen(false);
      }
      if (thinkingDropdownRef.current && !thinkingDropdownRef.current.contains(e.target as Node)) {
        setThinkingDropdownOpen(false);
      }
    }
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const [thinkingDropdownOpen, setThinkingDropdownOpen] = useState(false);

  // Auto-resize textarea
  useEffect(() => {
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
      textareaRef.current.style.height = Math.min(textareaRef.current.scrollHeight, 200) + 'px';
    }
  }, [inputValue]);

  function handleKeydown(event: React.KeyboardEvent) {
    if (event.key === 'Tab') {
      event.preventDefault();
      toggleMode();
      return;
    }
    if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      handleSubmit();
    }
  }

  async function handleSubmit() {
    const content = inputValue.trim();
    if (!content || !isInputEnabled || isSubmitting) return;

    setIsSubmitting(true);
    setStreaming(true);

    try {
      let sessionId = currentSessionId;

      // If draft session, create one first
      if (!sessionId) {
        const workspace = await api.getWorkspace();
        const { session_id } = await api.createSession({
          workspace_root: workspace.workspace_root,
          title: content.slice(0, 50),
        });
        sessionId = session_id;

        // Update store
        const [session, { messages }] = await Promise.all([
          api.getSession(sessionId),
          api.listMessages(sessionId),
        ]);
        commitDraftSession(session);
        setMessages(messages);
        setCurrentSessionId(sessionId);

        // Update URL
        const url = new URL(window.location.href);
        url.searchParams.set('session', sessionId);
        window.history.replaceState({}, '', url.toString());
      }

      // Send message
      const requestBody: {
        content: string;
        mode?: string;
        model_id?: string;
        provider_id?: string;
        thinking_level?: string;
      } = { content };

      if (mode) requestBody.mode = mode;
      if (selectedModelId) requestBody.model_id = selectedModelId;
      if (selectedProviderId) requestBody.provider_id = selectedProviderId;
      if (selectedThinking) requestBody.thinking_level = selectedThinking;

      const { request_id } = await api.sendMessage(sessionId, requestBody);
      setCurrentRequestId(request_id);
      setInputValue('');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to send message');
      setStreaming(false);
    } finally {
      setIsSubmitting(false);
    }
  }

  async function handleStop() {
    if (currentSessionId && currentRequestId) {
      try {
        await api.abortRequest(currentSessionId, { request_id: currentRequestId });
      } catch {
        // ignore
      }
      setStreaming(false);
      setCurrentRequestId(null);
    }
  }

  function handleModelSelect(model: ModelInfo) {
    setSelectedModelId(model.id);
    setSelectedProviderId(model.provider_id);
    setModelDropdownOpen(false);
    setModelSearchQuery('');
    updateThinkingLevels(model.id);
  }

  const selectedModelDisplay = selectedModelId
    ? models.find((m) => m.id === selectedModelId)
    : null;

  return (
    <div className="border-t border-neutral-200 bg-white px-4 py-3 dark:border-neutral-800 dark:bg-neutral-950">
      <div className="mx-auto flex max-w-4xl flex-col gap-2">
        {/* Controls row */}
        <div className="flex items-center gap-2">
          {/* Mode toggle */}
          <button
            onClick={toggleMode}
            className={`rounded px-2 py-1 text-xs font-medium transition-colors ${
              mode === 'plan'
                ? 'bg-purple-100 text-purple-700 hover:bg-purple-200 dark:bg-purple-900/30 dark:text-purple-300'
                : 'bg-emerald-100 text-emerald-700 hover:bg-emerald-200 dark:bg-emerald-900/30 dark:text-emerald-300'
            }`}
          >
            {mode === 'plan' ? 'Plan' : 'Build'}
          </button>

          {/* Model selector */}
          <div ref={dropdownRef} className="relative">
            <button
              onClick={() => setModelDropdownOpen(!modelDropdownOpen)}
              className="flex items-center gap-1 rounded bg-neutral-100 px-2 py-1 text-xs text-neutral-700 hover:bg-neutral-200 dark:bg-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-700"
            >
              <span className="max-w-[120px] truncate">
                {selectedModelDisplay?.display_name || 'Select model'}
              </span>
              <svg className="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
              </svg>
            </button>

            {modelDropdownOpen && (
              <div className="absolute bottom-full left-0 z-50 mb-1 w-72 rounded-lg border border-neutral-200 bg-white shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
                <div className="p-2">
                  <input
                    type="text"
                    value={modelSearchQuery}
                    onChange={(e) => setModelSearchQuery(e.target.value)}
                    placeholder="Search models..."
                    className="w-full rounded border border-neutral-300 px-2 py-1.5 text-xs dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-200"
                    autoFocus
                  />
                </div>
                <div className="max-h-64 overflow-y-auto">
                  {Array.from(groupedModels.entries()).map(([provider, providerModels]) => (
                    <div key={provider}>
                      <div className="px-3 py-1 text-xs font-medium text-neutral-500 dark:text-neutral-400">
                        {provider}
                      </div>
                      {providerModels.map((model) => (
                        <button
                          key={model.id}
                          onClick={() => handleModelSelect(model)}
                          className={`flex w-full items-center gap-2 px-3 py-2 text-left text-xs hover:bg-neutral-100 dark:hover:bg-neutral-800 ${selectedModelId === model.id ? 'bg-neutral-100 dark:bg-neutral-800' : ''}`}
                        >
                          <span className="flex-1 font-medium text-neutral-900 dark:text-neutral-100">
                            {model.display_name}
                          </span>
                          {model.supports_vision && (
                            <svg className="h-3.5 w-3.5 text-neutral-400" viewBox="0 0 24 24" fill="currentColor">
                              <path d="M12 15a3 3 0 100-6 3 3 0 000 6z" />
                              <path fillRule="evenodd" d="M1.323 11.447C2.811 6.976 7.028 3.75 12.001 3.75c4.97 0 9.185 3.223 10.675 7.69.12.362.12.752 0 1.113-1.487 4.471-5.705 7.697-10.677 7.697-4.97 0-9.186-3.223-10.675-7.69a1.762 1.762 0 010-1.113zM17.25 12a5.25 5.25 0 11-10.5 0 5.25 5.25 0 0110.5 0z" clipRule="evenodd" />
                            </svg>
                          )}
                        </button>
                      ))}
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>

          {/* Thinking level selector */}
          {thinkingOptions.length > 0 && (
            <div ref={thinkingDropdownRef} className="relative">
              <button
                onClick={() => setThinkingDropdownOpen(!thinkingDropdownOpen)}
                className="flex items-center gap-1 rounded bg-amber-50 px-2 py-1 text-xs text-amber-700 hover:bg-amber-100 dark:bg-amber-950/30 dark:text-amber-300 dark:hover:bg-amber-900/50"
              >
                <span>
                  {thinkingOptions.find((t) => t.value === selectedThinking)?.label || 'Thinking'}
                </span>
                <svg className="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                </svg>
              </button>

              {thinkingDropdownOpen && (
                <div className="absolute bottom-full left-0 z-50 mb-1 w-36 rounded-lg border border-neutral-200 bg-white shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
                  {thinkingOptions.map((option) => (
                    <button
                      key={option.value}
                      onClick={() => {
                        setSelectedThinking(option.value);
                        setThinkingDropdownOpen(false);
                      }}
                      className={`flex w-full px-3 py-2 text-left text-xs hover:bg-neutral-100 dark:hover:bg-neutral-800 ${selectedThinking === option.value ? 'bg-neutral-100 dark:bg-neutral-800 font-medium' : ''}`}
                    >
                      {option.label}
                    </button>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>

        {/* Input row */}
        <div className="flex items-end gap-2">
          <textarea
            ref={textareaRef}
            value={inputValue}
            onChange={(e) => setInputValue(e.target.value)}
            onKeyDown={handleKeydown}
            placeholder={
              isDraftSession
                ? 'Type your first message to create the session...'
                : currentSessionId
                  ? 'Type a message...'
                  : 'Select or create a session to start'
            }
            rows={1}
            disabled={!isInputEnabled}
            className="min-h-[44px] max-h-[200px] flex-1 resize-none rounded-xl border border-neutral-300 bg-white px-3 py-2.5 text-sm text-neutral-900 placeholder-neutral-400 outline-none transition-colors focus:border-neutral-500 focus:ring-1 focus:ring-neutral-500 disabled:opacity-50 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500 dark:focus:border-neutral-400"
          />

          {/* Send / Stop button */}
          {isStreaming ? (
            <button
              onClick={handleStop}
              className="mb-1 flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-full bg-red-600 text-white hover:bg-red-500"
              aria-label="Stop generating"
            >
              <svg className="h-4 w-4" fill="currentColor" viewBox="0 0 24 24">
                <rect x="6" y="6" width="12" height="12" rx="2" />
              </svg>
            </button>
          ) : (
            <button
              onClick={handleSubmit}
              disabled={!inputValue.trim() || !isInputEnabled || isSubmitting}
              className="mb-1 flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-full bg-neutral-900 text-white transition-colors hover:bg-neutral-800 disabled:opacity-50 disabled:hover:bg-neutral-900 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
              aria-label="Send message"
            >
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8" />
              </svg>
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
