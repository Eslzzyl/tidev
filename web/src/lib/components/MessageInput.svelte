<script lang="ts">
	import { sessionStore } from '../stores/session';
	import { uiStore } from '../stores/ui';
	import { api } from '../api/client';

	let inputValue = $state('');
	let isSubmitting = $state(false);
	let textareaRef: HTMLTextAreaElement | null = $state(null);

	function handleKeydown(event: KeyboardEvent) {
		if (event.key === 'Enter' && !event.shiftKey) {
			event.preventDefault();
			handleSubmit();
		}
	}

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
			// Send message
			await api.sendMessage(sessionId, { content });

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
			// Note: We'd need to track the request_id to abort properly
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
</script>

<div class="border-t border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
	<div class="mx-auto max-w-3xl">
		<div class="relative flex items-end gap-2 rounded-2xl border border-neutral-300 bg-white p-2 shadow-sm dark:border-neutral-700 dark:bg-neutral-900">
			<textarea
				bind:this={textareaRef}
				bind:value={inputValue}
				onkeydown={handleKeydown}
				oninput={autoResize}
				placeholder={placeholderText}
				rows="1"
				disabled={!isInputEnabled || isSubmitting}
				class="max-h-[200px] min-h-[44px] w-full resize-none rounded-xl border-0 bg-transparent px-3 py-2.5 text-sm text-neutral-900 placeholder:text-neutral-400 focus:outline-none focus:ring-0 disabled:opacity-50 dark:text-neutral-100"
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

		<div class="mt-2 text-center text-xs text-neutral-400 dark:text-neutral-600">
			{#if $sessionStore.isDraftSession}
				Your first message will create a new session
			{:else}
				Press Enter to send, Shift+Enter for new line
			{/if}
		</div>
	</div>
</div>
