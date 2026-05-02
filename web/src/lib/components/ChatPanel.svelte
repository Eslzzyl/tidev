<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { sessionStore } from '../stores/session';
	import { uiStore } from '../stores/ui';
	import { sseClient } from '../api/sse';
	import MessageBubble from './MessageBubble.svelte';
	import MessageInput from './MessageInput.svelte';

	let messagesEndRef: HTMLDivElement | null = $state(null);

	// SSE event handlers
	function handleMessageChunk(event: any) {
		// Handle streaming message chunks
		console.log('Message chunk:', event);
	}

	function handleMessageComplete(event: any) {
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

	// Auto-scroll to bottom when messages change
	$effect(() => {
		if ($sessionStore.messages && messagesEndRef) {
			messagesEndRef.scrollIntoView({ behavior: 'smooth' });
		}
	});

	// Connect SSE when session changes
	$effect(() => {
		if ($sessionStore.currentSessionId) {
			sseClient.connect($sessionStore.currentSessionId);
			uiStore.setConnectionStatus('connecting');
		}
	});
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

			{#if $sessionStore.currentSession}
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

		<!-- Connection Status -->
		<div class="flex items-center gap-2">
			<div
				class="h-2 w-2 rounded-full {$uiStore.connectionStatus === 'connected'
					? 'bg-green-500'
					: $uiStore.connectionStatus === 'connecting'
						? 'bg-amber-500'
						: 'bg-red-500'}"
			></div>
			<span class="hidden text-xs text-neutral-500 sm:inline dark:text-neutral-400">
				{$uiStore.connectionStatus}
			</span>
		</div>
	</div>

	<!-- Messages Area -->
	<div class="flex-1 overflow-y-auto">
		{#if !$sessionStore.currentSessionId}
			<div class="flex h-full items-center justify-center">
				<div class="text-center">
					<p class="text-neutral-500 dark:text-neutral-400">Select a session or create a new one to start chatting</p>
				</div>
			</div>
		{:else if $sessionStore.messages.length === 0}
			<div class="flex h-full items-center justify-center">
				<div class="text-center">
					<p class="text-neutral-500 dark:text-neutral-400">No messages yet. Start a conversation!</p>
				</div>
			</div>
		{:else}
			<div class="divide-y divide-neutral-100 dark:divide-neutral-900">
				{#each $sessionStore.messages as message (message.id)}
					<MessageBubble {message} />
				{/each}
			</div>
		{/if}
		<div bind:this={messagesEndRef}></div>
	</div>

	<!-- Input Area -->
	<MessageInput />
</div>
