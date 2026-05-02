<script lang="ts">
	import { onMount } from 'svelte';
	import { sessionStore } from './lib/stores/session';
	import { uiStore, effectiveTheme } from './lib/stores/ui';
	import { api } from './lib/api/client';
	import SessionList from './lib/components/SessionList.svelte';
	import ChatPanel from './lib/components/ChatPanel.svelte';

	let isLoading = $state(true);
	let loadError = $state<string | null>(null);

	onMount(async () => {
		try {
			// Load sessions
			const { sessions } = await api.listSessions();
			sessionStore.setSessions(sessions);

			// If there's a session in URL, load it
			const params = new URLSearchParams(window.location.search);
			const sessionId = params.get('session');
			if (sessionId) {
				const [session, { messages }] = await Promise.all([
					api.getSession(sessionId),
					api.listMessages(sessionId)
				]);
				sessionStore.setCurrentSession(session);
				sessionStore.setMessages(messages);
			}
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'Failed to load sessions';
		} finally {
			isLoading = false;
		}

		// Apply theme
		const applyTheme = () => {
			const theme = $effectiveTheme;
			if (theme === 'dark') {
				document.documentElement.classList.add('dark');
			} else {
				document.documentElement.classList.remove('dark');
			}
		};

		applyTheme();

		// Listen for system theme changes
		const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
		mediaQuery.addEventListener('change', applyTheme);

		return () => {
			mediaQuery.removeEventListener('change', applyTheme);
		};
	});
</script>

<div class="h-screen w-full bg-white dark:bg-neutral-950">
	{#if isLoading}
		<div class="flex h-full items-center justify-center">
			<div class="text-center">
				<div class="mb-4 h-8 w-8 animate-spin rounded-full border-2 border-neutral-300 border-t-neutral-900 dark:border-neutral-700 dark:border-t-neutral-100"></div>
				<p class="text-sm text-neutral-600 dark:text-neutral-400">Loading...</p>
			</div>
		</div>
	{:else if loadError}
		<div class="flex h-full items-center justify-center">
			<div class="text-center">
				<p class="mb-2 text-red-600 dark:text-red-400">{loadError}</p>
				<button
					onclick={() => window.location.reload()}
					class="rounded bg-neutral-900 px-4 py-2 text-sm font-medium text-white hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
				>
					Retry
				</button>
			</div>
		</div>
	{:else}
		<div class="flex h-full">
			<!-- Sidebar - Hidden on mobile by default -->
			<aside
				class="fixed inset-y-0 left-0 z-50 w-64 transform border-r border-neutral-200 bg-white transition-transform duration-200 ease-in-out md:relative md:translate-x-0 dark:border-neutral-800 dark:bg-neutral-950 {$uiStore.mobileMenuOpen
					? 'translate-x-0'
					: '-translate-x-full'}"
			>
				<SessionList />
			</aside>

			<!-- Mobile overlay -->
			{#if $uiStore.mobileMenuOpen}
				<button
					onclick={() => uiStore.closeMobileMenu()}
					class="fixed inset-0 z-40 bg-black/50 md:hidden"
					aria-label="Close menu"
				></button>
			{/if}

			<!-- Main content -->
			<main class="flex-1 min-w-0">
				<ChatPanel />
			</main>
		</div>
	{/if}
</div>
