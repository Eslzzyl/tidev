<script lang="ts">
	import { onMount } from 'svelte';
	import { sessionStore } from '../stores/session';
	import { uiStore } from '../stores/ui';
	import { api, type ModelInfo } from '../api/client';

	let inputValue = $state('');
	let isSubmitting = $state(false);
	let textareaRef: HTMLTextAreaElement | null = $state(null);

	function toggleMode() {
		sessionStore.toggleMode();
	}

	function handleKeydown(event: KeyboardEvent) {
		if (event.key === 'Tab') {
			event.preventDefault();
			toggleMode();
			return;
		}
		if (event.key === 'Enter' && !event.shiftKey) {
			event.preventDefault();
			handleSubmit();
		}
	}

	// Models state
	let models: ModelInfo[] = $state([]);
	let modelDropdownOpen = $state(false);
	let modelSearchQuery = $state('');

	// Selected model
	let selectedModelId = $state<string | null>(null);
	let selectedProviderId = $state<string | null>(null);

	// Thinking level
	type ThinkingOption = { label: string; value: string };
	let thinkingOptions: ThinkingOption[] = $state([]);
	let selectedThinking = $state<string>('');

	// Filtered models based on search query
	let filteredModels = $derived.by(() => {
		if (!modelSearchQuery.trim()) return models;
		const q = modelSearchQuery.toLowerCase();
		return models.filter(
			(m) =>
				m.display_name.toLowerCase().includes(q) ||
				m.id.toLowerCase().includes(q) ||
				m.provider_name.toLowerCase().includes(q)
		);
	});

	// Group filtered models by provider
	let groupedModels = $derived.by(() => {
		const groups = new Map<string, ModelInfo[]>();
		for (const m of filteredModels) {
			const key = m.provider_name || m.provider_id;
			if (!groups.has(key)) {
				groups.set(key, []);
			}
			groups.get(key)!.push(m);
		}
		return groups;
	});

	// Determine thinking levels based on model
	function updateThinkingLevels(modelId: string) {
		const id = modelId.toLowerCase();
		if (id.includes('deepseek') && id.includes('4')) {
			thinkingOptions = [
				{ label: 'Off', value: 'deepseek:Off' },
				{ label: 'High', value: 'deepseek:High' },
				{ label: 'Max', value: 'deepseek:Max' }
			];
		} else if (id.includes('qwen') && id.includes('3.')) {
			thinkingOptions = [
				{ label: 'Off', value: 'qwen:Off' },
				{ label: 'On', value: 'qwen:On' }
			];
		} else if (id.includes('glm')) {
			thinkingOptions = [
				{ label: 'Off', value: 'glm:Off' },
				{ label: 'On', value: 'glm:On' }
			];
		} else {
			thinkingOptions = [];
		}
		// Default to first option if available
		if (thinkingOptions.length > 0) {
			selectedThinking = thinkingOptions[0].value;
		} else {
			selectedThinking = '';
		}
	}

	function selectModel(modelId: string, providerId: string) {
		selectedModelId = modelId;
		selectedProviderId = providerId;
		updateThinkingLevels(modelId);
		modelDropdownOpen = false;
		modelSearchQuery = '';
	}

	// Get current model display name
	let currentModelLabel = $derived.by(() => {
		if (selectedModelId) {
			const m = models.find((m) => m.id === selectedModelId);
			return m ? m.display_name : selectedModelId;
		}
		if ($sessionStore.currentSession) {
			return $sessionStore.currentSession.model_display_name;
		}
		return 'Select model';
	});

	// Load models on mount
	onMount(async () => {
		try {
			const { models: modelList } = await api.listModels();
			models = modelList;
			// Select first model as default if none selected
			if (modelList.length > 0 && !selectedModelId) {
				selectedModelId = modelList[0].id;
				selectedProviderId = modelList[0].provider_id;
				updateThinkingLevels(modelList[0].id);
			}
		} catch {
			// Silently fail - models endpoint may not be available
		}
	});

	function autoResize() {
		if (textareaRef) {
			textareaRef.style.height = 'auto';
			textareaRef.style.height = Math.min(textareaRef.scrollHeight, 200) + 'px';
		}
	}

	async function handleSubmit() {
		if (!inputValue.trim() || isSubmitting) return;

		// Handle draft session - create session on first message
		if ($sessionStore.isDraftSession) {
			await handleCreateSessionAndSendMessage();
			return;
		}

		if (!$sessionStore.currentSessionId) {
			sessionStore.setError('No active session');
			return;
		}

		await sendMessageToSession($sessionStore.currentSessionId);
	}

	async function handleCreateSessionAndSendMessage() {
		isSubmitting = true;
		const content = inputValue.trim();

		try {
			// Get workspace info
			const workspace = await api.getWorkspace();

			// Create session with first message as title (truncated)
			const title = content.length > 50 ? content.slice(0, 50) + '...' : content;

			const { session_id } = await api.createSession({
				workspace_root: workspace.workspace_root,
				title
			});

			// Get the new session details
			const session = await api.getSession(session_id);
			sessionStore.commitDraftSession(session);

			// Update sessions list
			const { sessions } = await api.listSessions();
			sessionStore.setSessions(sessions);

			// Clear input and resize
			inputValue = '';
			if (textareaRef) textareaRef.style.height = 'auto';

			// Send the first message
			await sendMessageToSession(session_id);
		} catch (err) {
			sessionStore.setError(err instanceof Error ? err.message : 'Failed to create session');
		} finally {
			isSubmitting = false;
		}
	}

	async function sendMessageToSession(sessionId: string) {
		const content = inputValue.trim();
		if (!content) return;

		inputValue = '';
		if (textareaRef) textareaRef.style.height = 'auto';

		isSubmitting = true;
		uiStore.setStreaming(true);

		try {
			// Build request with optional model override, thinking level, and mode
			const request: { content: string; thinking_level?: string; model_id?: string; provider_id?: string; mode?: string } = { content };
			if (selectedThinking) {
				request.thinking_level = selectedThinking;
			}
			if (selectedModelId && selectedProviderId) {
				request.model_id = selectedModelId;
				request.provider_id = selectedProviderId;
			}
			request.mode = $sessionStore.mode;

			// Send message
			await api.sendMessage(sessionId, request);

			// Refresh messages
			const { messages } = await api.listMessages(sessionId);
			sessionStore.setMessages(messages);
		} catch (err) {
			sessionStore.setError(err instanceof Error ? err.message : 'Failed to send message');
		} finally {
			isSubmitting = false;
			uiStore.setStreaming(false);
		}
	}

	function handleAbort() {
		if ($sessionStore.currentSessionId && $uiStore.isStreaming) {
			uiStore.setStreaming(false);
		}
	}

	// Get placeholder text based on session state
	let placeholderText = $state('Select a session or click New to start chatting');

	$effect(() => {
		if ($sessionStore.isDraftSession) {
			placeholderText = 'Type your first message to create a new session...';
		} else if ($sessionStore.currentSessionId) {
			placeholderText = 'Type a message...';
		} else {
			placeholderText = 'Select a session or click New to start chatting';
		}
	});

	// Check if input is enabled
	let isInputEnabled = $derived($sessionStore.currentSessionId !== null || $sessionStore.isDraftSession);

	// Close dropdown on click outside
	function handleClickOutside(e: MouseEvent) {
		if (modelDropdownOpen) {
			const target = e.target as HTMLElement;
			if (!target.closest('[data-model-selector]')) {
				modelDropdownOpen = false;
				modelSearchQuery = '';
			}
		}
	}
</script>

<svelte:window onclick={handleClickOutside} />

<div class="border-t border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-950">
	<div class="mx-auto max-w-3xl px-4 pb-4 pt-3">
		<!-- Model Selector and Thinking Level Selector -->
		<div class="mb-2 flex items-center gap-3" data-model-selector>
			<!-- Mode Selector (Plan/Build toggle) -->
			<button
				onclick={toggleMode}
				disabled={!isInputEnabled || isSubmitting}
				class={`flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium disabled:opacity-40 ${
					$sessionStore.mode === 'plan'
						? 'border-amber-300 bg-amber-50 text-amber-700 dark:border-amber-700 dark:bg-amber-950/40 dark:text-amber-300'
						: 'border-emerald-300 bg-emerald-50 text-emerald-700 dark:border-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300'
				}`}
				title={$sessionStore.mode === 'plan' ? 'Plan mode — read-only' : 'Build mode — full access'}
			>
				{#if $sessionStore.mode === 'plan'}
					<svg class="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
						<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
					</svg>
				{:else}
					<svg class="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
						<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
					</svg>
				{/if}
				<span>{$sessionStore.mode === 'plan' ? 'Plan' : 'Build'}</span>
			</button>

			<!-- Model Selector Dropdown -->
			<div class="relative">
				<button
					onclick={() => { modelDropdownOpen = !modelDropdownOpen; if (modelDropdownOpen) modelSearchQuery = ''; }}
					disabled={!isInputEnabled || isSubmitting}
					class="flex items-center gap-1.5 rounded-lg border border-neutral-200 bg-white px-2.5 py-1.5 text-xs text-neutral-600 hover:bg-neutral-50 disabled:opacity-40 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-700"
				>
					<svg class="h-3.5 w-3.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
						<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2z" />
					</svg>
					<span class="max-w-[140px] truncate">{currentModelLabel}</span>
					<svg class={`h-3 w-3 transition-transform ${modelDropdownOpen ? 'rotate-180' : ''}`} fill="none" stroke="currentColor" viewBox="0 0 24 24">
						<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" />
					</svg>
				</button>

				{#if modelDropdownOpen}
					<div class="absolute bottom-full left-0 z-50 mb-1 w-[240px] rounded-lg border border-neutral-200 bg-white shadow-lg dark:border-neutral-700 dark:bg-neutral-800">
						<!-- Search box -->
						<div class="border-b border-neutral-100 p-2 dark:border-neutral-700">
							<div class="flex items-center gap-1.5 rounded-md border border-neutral-200 bg-neutral-50 px-2 py-1.5 dark:border-neutral-600 dark:bg-neutral-700">
								<svg class="h-3.5 w-3.5 flex-shrink-0 text-neutral-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
									<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
								</svg>
								<input
									type="text"
									bind:value={modelSearchQuery}
									placeholder="Search models..."
									style="outline: none !important"
									class="w-full border-0 bg-transparent text-xs text-neutral-700 placeholder:text-neutral-400 focus:outline-none focus-visible:outline-none focus:ring-0 dark:text-neutral-200 dark:placeholder:text-neutral-500"
								/>
							</div>
						</div>
						<!-- Model list -->
						<div class="max-h-[280px] overflow-y-auto">
							{#each [...groupedModels.entries()] as [providerName, providerModels]}
								<div class="border-b border-neutral-100 last:border-b-0 dark:border-neutral-700">
									<div class="px-3 py-1.5 text-[11px] font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
										{providerName}
									</div>
									{#each providerModels as model}
										<button
											onclick={() => selectModel(model.id, model.provider_id)}
											class={`flex w-full items-center gap-2 px-3 py-2 text-left text-xs transition-colors hover:bg-neutral-100 dark:hover:bg-neutral-700 ${model.id === selectedModelId ? 'bg-blue-50 text-blue-700 dark:bg-blue-950/40 dark:text-blue-300' : 'text-neutral-700 dark:text-neutral-300'}`}
										>
											<div class="min-w-0 flex-1 truncate font-medium">{model.display_name}</div>
											{#if model.supports_vision}
												<span class="flex-shrink-0 text-neutral-400 dark:text-neutral-500" title="Supports vision">
													<svg class="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
														<path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
														<path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
													</svg>
												</span>
											{/if}
										</button>
									{/each}
								</div>
							{/each}
							{#if filteredModels.length === 0}
								<div class="px-3 py-4 text-center text-xs text-neutral-400 dark:text-neutral-500">
									No models found
								</div>
							{/if}
						</div>
					</div>
				{/if}
			</div>

			<!-- Thinking Level Selector -->
			{#if thinkingOptions.length > 0}
				<div class="flex items-center gap-1.5 text-xs text-neutral-500 dark:text-neutral-400">
					<svg class="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
						<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z" />
					</svg>
					<select
						bind:value={selectedThinking}
						disabled={!isInputEnabled || isSubmitting}
						class="rounded-lg border border-neutral-200 bg-white px-2 py-1 text-xs text-neutral-600 disabled:opacity-40 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-300"
					>
						{#each thinkingOptions as opt}
							<option value={opt.value}>{opt.label}</option>
						{/each}
					</select>
				</div>
			{/if}
		</div>

		<!-- Input Area -->
		<div class="relative flex items-end gap-2 rounded-2xl border border-neutral-300 bg-white p-2 shadow-sm dark:border-neutral-700 dark:bg-neutral-900">
			<textarea
				bind:this={textareaRef}
				bind:value={inputValue}
				onkeydown={handleKeydown}
				oninput={autoResize}
				placeholder={placeholderText}
				rows="1"
				disabled={!isInputEnabled || isSubmitting}
				style="outline: none !important"
				class="max-h-[200px] min-h-[44px] w-full resize-none rounded-xl border-0 bg-transparent px-3 py-2.5 text-sm text-neutral-900 placeholder:text-neutral-400 focus:outline-none focus-visible:outline-none focus:ring-0 disabled:opacity-50 dark:text-neutral-100"
			></textarea>

			{#if $uiStore.isStreaming}
				<button
					onclick={handleAbort}
					class="mb-1 flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-full bg-red-100 text-red-600 hover:bg-red-200 dark:bg-red-950 dark:text-red-400"
					aria-label="Stop generating"
				>
					<svg class="h-4 w-4" fill="currentColor" viewBox="0 0 24 24">
						<rect x="6" y="6" width="12" height="12" rx="2" />
					</svg>
				</button>
			{:else}
				<button
					onclick={handleSubmit}
					disabled={!inputValue.trim() || !isInputEnabled || isSubmitting}
					class="mb-1 flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-full bg-neutral-900 text-white transition-colors hover:bg-neutral-800 disabled:opacity-50 disabled:hover:bg-neutral-900 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
					aria-label="Send message"
				>
					<svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
						<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8" />
					</svg>
				</button>
			{/if}
		</div>
	</div>
</div>
