<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { sessionStore } from '../stores/session';
	import { uiStore } from '../stores/ui';
	import { sseClient } from '../api/sse';
	import type { Message } from '../api/client';
	import MessageBubble from './MessageBubble.svelte';
	import MessageInput from './MessageInput.svelte';

	let messagesContainerRef: HTMLDivElement | null = $state(null);
	let messagesEndRef: HTMLDivElement | null = $state(null);
	let shouldAutoScroll = $state(true);
	let isFirstLoad = $state(true);

	// Group messages: associate tool results with their assistant messages
	let displayMessages: Message[] = $state([]);
	let toolResultsByAssistant = $state(new Map<string, Message[]>());

	$effect(() => {
		const messages = $sessionStore.messages;
		const toolResults = new Map<string, Message>();
		const toolCallToAssistant = new Map<string, string>(); // tool_call_id -> assistant message id

		// First pass: collect tool results and map tool calls to assistant messages
		for (const msg of messages) {
			if (msg.role === 'tool' && msg.tool_call_id) {
				toolResults.set(msg.tool_call_id, msg);
			}
		}

		// Find which assistant message each tool call belongs to
		for (const msg of messages) {
			if (msg.role === 'assistant' && msg.tool_calls) {
				for (const tc of msg.tool_calls) {
					toolCallToAssistant.set(tc.id, msg.id);
				}
			}
		}

		// Group tool results by assistant message
		const grouped = new Map<string, Message[]>();
		for (const [toolCallId, resultMsg] of toolResults) {
			const assistantId = toolCallToAssistant.get(toolCallId);
			if (assistantId) {
				if (!grouped.has(assistantId)) {
					grouped.set(assistantId, []);
				}
				grouped.get(assistantId)!.push(resultMsg);
			}
		}

		// Filter out standalone tool messages (they're now grouped with assistants)
		// Keep tool messages that couldn't be associated with an assistant
		const standaloneTools: Message[] = [];
		for (const msg of messages) {
			if (msg.role === 'tool' && msg.tool_call_id) {
				const assistantId = toolCallToAssistant.get(msg.tool_call_id);
				if (!assistantId) {
					standaloneTools.push(msg);
				}
			}
		}

		// Build final list: non-tool messages + orphaned tool messages
		const nonToolMessages = messages.filter(m => m.role !== 'tool');

		displayMessages = [...nonToolMessages, ...standaloneTools].sort(
			(a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime()
		);
		toolResultsByAssistant = grouped;
	});

	// eslint-disable-next-line @typescript-eslint/no-unused-vars
	function handleMessageChunk(_event: unknown) {
		// Handle streaming message chunks - currently just logging
	}

	// eslint-disable-next-line @typescript-eslint/no-unused-vars
	function handleMessageComplete(_event: unknown) {
		uiStore.setStreaming(false);
		// Refresh messages to get the complete message
		if ($sessionStore.currentSessionId) {
			import('../api/client').then(({ api }) => {
				api.listMessages($sessionStore.currentSessionId!).then(({ messages }) => {
					sessionStore.setMessages(messages);
				});
			});
		}
	}

	function handleConnected() {
		uiStore.setConnectionStatus('connected');
	}

	function handleError() {
		uiStore.setConnectionStatus('disconnected');
	}

	onMount(() => {
		// Setup SSE listeners
		sseClient.on('message.chunk', handleMessageChunk);
		sseClient.on('message.complete', handleMessageComplete);
		sseClient.on('connected', handleConnected);
		sseClient.on('error', handleError);

		// Connect SSE if we have a session
		if ($sessionStore.currentSessionId) {
			sseClient.connect($sessionStore.currentSessionId);
		}
	});

	onDestroy(() => {
		sseClient.off('message.chunk', handleMessageChunk);
		sseClient.off('message.complete', handleMessageComplete);
		sseClient.off('connected', handleConnected);
		sseClient.off('error', handleError);
		sseClient.disconnect();
	});

	// Scroll to bottom on initial load without animation
	$effect(() => {
		if (isFirstLoad && $sessionStore.messages.length > 0 && messagesEndRef) {
			// Use instant scroll on first load
			messagesEndRef.scrollIntoView({ behavior: 'instant' });
			isFirstLoad = false;
		}
	});

	// Auto-scroll on new messages (with smooth behavior)
	$effect(() => {
		const messageCount = $sessionStore.messages.length;
		if (messageCount > 0 && messagesEndRef && !isFirstLoad && shouldAutoScroll) {
			// Use requestAnimationFrame to ensure DOM is updated
			requestAnimationFrame(() => {
				messagesEndRef?.scrollIntoView({ behavior: 'smooth' });
			});
		}
	});

	// Reset first load flag when session changes
	$effect(() => {
		if ($sessionStore.currentSessionId) {
			isFirstLoad = true;
			// Connect SSE when session changes
			sseClient.connect($sessionStore.currentSessionId);
			uiStore.setConnectionStatus('connecting');
		}
	});

	// Handle scroll to detect if user manually scrolled up
function handleScroll() {
		const scrollHeight = messagesContainerRef?.scrollHeight ?? 0;
		const scrollTop = messagesContainerRef?.scrollTop ?? 0;
		const clientHeight = messagesContainerRef?.clientHeight ?? 0;
		const isNearBottom = scrollHeight - scrollTop - clientHeight < 100;
		shouldAutoScroll = isNearBottom;
	}
	function scrollToBottom() {
		if (messagesEndRef) {
			messagesEndRef.scrollIntoView({ behavior: 'smooth' });
			shouldAutoScroll = true;
		}
	}
</script>

<div class="flex h-full flex-col bg-white dark:bg-neutral-950">
	<!-- Header -->
	<div class="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
		<div class="flex items-center gap-3">
			<button
				onclick={() => uiStore.toggleMobileMenu()}
				class="rounded p-1 text-neutral-600 hover:bg-neutral-100 md:hidden dark:text-neutral-400 dark:hover:bg-neutral-800"
				aria-label="Open menu"
			>
				<svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h16M4 12h16M4 18h16" />
				</svg>
			</button>

			{#if $sessionStore.isDraftSession}
				<div>
					<h1 class="text-sm font-semibold text-blue-600 dark:text-blue-400">
						{$sessionStore.draftTitle}
					</h1>
					<p class="text-xs text-neutral-500 dark:text-neutral-400">
						Draft Session
					</p>
				</div>
			{:else if $sessionStore.currentSession}
				<div>
					<h1 class="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
						{$sessionStore.currentSession.title}
					</h1>
					<p class="text-xs text-neutral-500 dark:text-neutral-400">
						{$sessionStore.currentSession.model_display_name}
					</p>
				</div>
			{:else}
				<h1 class="text-sm font-semibold text-neutral-900 dark:text-neutral-100">Select a session</h1>
			{/if}
		</div>

		<div class="flex items-center gap-2">
			<!-- Settings button -->
			<button
				onclick={() => uiStore.toggleSettings()}
				class="rounded p-2 text-neutral-600 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
				aria-label="Settings"
			>
				<svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
				</svg>
			</button>

			<!-- Right sidebar toggle (desktop) -->
			<button
				onclick={() => uiStore.toggleRightSidebar()}
				class="hidden rounded p-2 text-neutral-600 hover:bg-neutral-100 md:block dark:text-neutral-400 dark:hover:bg-neutral-800"
				aria-label="Toggle info panel"
			>
				<svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
				</svg>
			</button>

			<!-- Mobile right sidebar toggle -->
			<button
				onclick={() => uiStore.toggleMobileRightSidebar()}
				class="rounded p-2 text-neutral-600 hover:bg-neutral-100 md:hidden dark:text-neutral-400 dark:hover:bg-neutral-800"
				aria-label="Open info panel"
			>
				<svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
				</svg>
			</button>

		</div>
	</div>

	<!-- Messages Area -->
	<div
		bind:this={messagesContainerRef}
		onscroll={handleScroll}
		class="flex-1 overflow-y-auto"
	>
		{#if !$sessionStore.currentSessionId && !$sessionStore.isDraftSession}
			<div class="flex h-full items-center justify-center">
				<div class="text-center">
					<p class="text-neutral-500 dark:text-neutral-400">Select a session or create a new one to start chatting</p>
				</div>
			</div>
		{:else if $sessionStore.messages.length === 0}
			<div class="flex h-full items-center justify-center">
				<div class="text-center">
					<p class="text-neutral-500 dark:text-neutral-400">
						{$sessionStore.isDraftSession
							? 'Type your first message to create the session'
							: 'No messages yet. Start a conversation!'}
					</p>
				</div>
			</div>
		{:else}
			<div class="divide-y divide-neutral-100 dark:divide-neutral-900">
				{#each displayMessages as message (message.id)}
					<MessageBubble
						{message}
						toolResults={toolResultsByAssistant.get(message.id) || []}
					/>
				{/each}
			</div>
		{/if}
		<div bind:this={messagesEndRef}></div>
	</div>

	<!-- Input Area -->
	<MessageInput />
</div>
