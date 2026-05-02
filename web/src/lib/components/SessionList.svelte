<script lang="ts">
	import { onMount } from 'svelte';
	import { sessionStore } from '../stores/session';
	import { uiStore } from '../stores/ui';
	import { api } from '../api/client';

	let isCreating = $state(false);
	let newSessionTitle = $state('');
	let showNewSessionInput = $state(false);
	let workspaceRoot = $state('');

	// Load workspace info on mount
	onMount(() => {
		api.getWorkspace().then((workspace) => {
			workspaceRoot = workspace.workspace_root;
		}).catch((err) => {
			console.error('Failed to load workspace:', err);
		});
	});

	async function handleCreateSession() {
		if (!newSessionTitle.trim()) return;

		isCreating = true;
		try {
			const response = await api.createSession({
				workspace_root: workspaceRoot || '.',
				title: newSessionTitle.trim()
			});
			const { sessions } = await api.listSessions();
			sessionStore.setSessions(sessions);
			const session = await api.getSession(response.session_id);
			sessionStore.setCurrentSession(session);
			const { messages } = await api.listMessages(response.session_id);
			sessionStore.setMessages(messages);
			newSessionTitle = '';
			showNewSessionInput = false;
			uiStore.closeMobileMenu();
		} catch (err) {
			sessionStore.setError(err instanceof Error ? err.message : 'Failed to create session');
		} finally {
			isCreating = false;
		}
	}

	async function handleSelectSession(sessionId: string) {
		try {
			uiStore.setLoading(true);
			const [session, { messages }] = await Promise.all([
				api.getSession(sessionId),
				api.listMessages(sessionId)
			]);
			sessionStore.setCurrentSession(session);
			sessionStore.setMessages(messages);
			uiStore.closeMobileMenu();
		} catch (err) {
			sessionStore.setError(err instanceof Error ? err.message : 'Failed to load session');
		} finally {
			uiStore.setLoading(false);
		}
	}

	async function handleDeleteSession(sessionId: string) {
		if (!confirm('Delete this session?')) return;
		try {
			await api.deleteSession(sessionId);
			sessionStore.removeSession(sessionId);
		} catch (err) {
			sessionStore.setError(err instanceof Error ? err.message : 'Failed to delete session');
		}
	}

	function formatDate(dateStr: string): string {
		const date = new Date(dateStr);
		const now = new Date();
		const diff = now.getTime() - date.getTime();
		const days = Math.floor(diff / (1000 * 60 * 60 * 24));

		if (days === 0) {
			return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
		} else if (days === 1) {
			return 'Yesterday';
		} else if (days < 7) {
			return date.toLocaleDateString([], { weekday: 'short' });
		} else {
			return date.toLocaleDateString([], { month: 'short', day: 'numeric' });
		}
	}
</script>

<div class="flex h-full flex-col bg-neutral-50 dark:bg-neutral-900">
	<!-- Header -->
	<div class="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
		<h2 class="text-sm font-semibold text-neutral-900 dark:text-neutral-100">Sessions</h2>
		<button
			onclick={() => (showNewSessionInput = !showNewSessionInput)}
			class="rounded p-1 text-neutral-600 hover:bg-neutral-200 dark:text-neutral-400 dark:hover:bg-neutral-800"
			aria-label="New session"
		>
			<svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
				<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
			</svg>
		</button>
	</div>

	<!-- New Session Input -->
	{#if showNewSessionInput}
		<div class="border-b border-neutral-200 p-3 dark:border-neutral-800">
			<form onsubmit={(e) => { e.preventDefault(); handleCreateSession(); }} class="flex gap-2">
				<input
					type="text"
					bind:value={newSessionTitle}
					placeholder="Session name..."
					class="flex-1 rounded border border-neutral-300 bg-white px-3 py-1.5 text-sm focus:border-neutral-500 focus:outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
				/>
				<button
					type="submit"
					disabled={isCreating || !newSessionTitle.trim()}
					class="rounded bg-neutral-900 px-3 py-1.5 text-sm font-medium text-white hover:bg-neutral-800 disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
				>
					{isCreating ? '...' : 'Create'}
				</button>
			</form>
		</div>
	{/if}

	<!-- Session List -->
	<div class="flex-1 overflow-y-auto">
		{#if $sessionStore.sessions.length === 0}
			<div class="p-4 text-center text-sm text-neutral-500 dark:text-neutral-400">
				No sessions yet
			</div>
		{:else}
			<ul class="divide-y divide-neutral-100 dark:divide-neutral-800">
				{#each $sessionStore.sessions as session (session.session_id)}
					<li class="group relative">
						<button
							onclick={() => handleSelectSession(session.session_id)}
							class="flex w-full items-center px-4 py-3 text-left hover:bg-neutral-100 dark:hover:bg-neutral-800 {$sessionStore.currentSessionId === session.session_id ? 'bg-neutral-100 dark:bg-neutral-800' : ''}"
						>
							<div class="min-w-0 flex-1 pr-8">
								<p class="truncate text-sm font-medium text-neutral-900 dark:text-neutral-100">
									{session.title}
								</p>
								<p class="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
									{session.model_display_name} • {formatDate(session.updated_at)}
								</p>
							</div>
						</button>
						<button
							onclick={() => handleDeleteSession(session.session_id)}
							class="absolute right-3 top-1/2 -translate-y-1/2 rounded p-1 opacity-0 text-neutral-400 hover:text-red-600 group-hover:opacity-100 dark:text-neutral-500 dark:hover:text-red-400"
							aria-label="Delete session"
						>
							<svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
								<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
							</svg>
						</button>
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</div>
