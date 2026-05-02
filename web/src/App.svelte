<script lang="ts">
	import { onMount } from 'svelte';
	import { sessionStore } from './lib/stores/session';
	import { uiStore, effectiveTheme } from './lib/stores/ui';
	import { api } from './lib/api/client';
	import SessionList from './lib/components/SessionList.svelte';
	import ChatPanel from './lib/components/ChatPanel.svelte';
	import RightSidebar from './lib/components/RightSidebar.svelte';
	import Settings from './lib/components/Settings.svelte';

	let isLoading = $state(true);
	let loadError = $state<string | null>(null);

	// Resizing state
	let isResizingLeft = $state(false);
	let isResizingRight = $state(false);
	let startX = $state(0);
	let startWidth = $state(0);

	onMount(() => {
		// Load sessions and setup
		const loadData = async () => {
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
		};

		loadData();

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

	// Handle left sidebar resize start
	function handleLeftResizeStart(e: MouseEvent) {
		isResizingLeft = true;
		startX = e.clientX;
		startWidth = $uiStore.leftSidebarWidth;
		document.body.style.cursor = 'col-resize';
		document.body.style.userSelect = 'none';
	}

	// Handle right sidebar resize start
	function handleRightResizeStart(e: MouseEvent) {
		isResizingRight = true;
		startX = e.clientX;
		startWidth = $uiStore.rightSidebarWidth;
		document.body.style.cursor = 'col-resize';
		document.body.style.userSelect = 'none';
	}

	// Handle resize move
	function handleResizeMove(e: MouseEvent) {
		if (isResizingLeft) {
			const diff = e.clientX - startX;
			uiStore.setLeftSidebarWidth(startWidth + diff);
		} else if (isResizingRight) {
			const diff = startX - e.clientX;
			uiStore.setRightSidebarWidth(startWidth + diff);
		}
	}

	// Handle resize end
	function handleResizeEnd() {
		isResizingLeft = false;
		isResizingRight = false;
		document.body.style.cursor = '';
		document.body.style.userSelect = '';
	}
</script>

<svelte:window
	onmousemove={handleResizeMove}
	onmouseup={handleResizeEnd}
/>

<!-- Settings Modal -->
<Settings />

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
			<!-- Left Sidebar - Hidden on mobile by default -->
			<aside
				class="fixed inset-y-0 left-0 z-50 transform border-r border-neutral-200 bg-white transition-transform duration-200 ease-in-out md:relative md:translate-x-0 dark:border-neutral-800 dark:bg-neutral-950 {$uiStore.mobileMenuOpen
					? 'translate-x-0'
					: '-translate-x-full'}"
				style="width: {$uiStore.leftSidebarWidth}px"
			>
				<SessionList />
			</aside>

			<!-- Left Sidebar Resize Handle (desktop only) -->
			<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
			<div
				class="hidden w-1 cursor-col-resize bg-transparent hover:bg-neutral-300 dark:hover:bg-neutral-700 md:block {$uiStore.mobileMenuOpen ? 'hidden' : ''}"
				class:bg-neutral-400={isResizingLeft}
				class:dark:bg-neutral-600={isResizingLeft}
				onmousedown={handleLeftResizeStart}
				role="separator"
				aria-label="Resize left sidebar"
			></div>

			<!-- Mobile overlay for left sidebar -->
			{#if $uiStore.mobileMenuOpen}
				<button
					onclick={() => uiStore.closeMobileMenu()}
					class="fixed inset-0 z-40 bg-black/50 md:hidden"
					aria-label="Close menu"
				></button>
			{/if}

			<!-- Main content -->
			<main class="relative flex-1 min-w-0">
				<ChatPanel />
			</main>

			<!-- Right Sidebar (desktop only, collapsible) -->
			{#if $uiStore.rightSidebarOpen}
				<!-- Right Sidebar Resize Handle -->
				<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
				<div
					class="hidden w-1 cursor-col-resize border-l border-neutral-200 bg-transparent hover:bg-neutral-300 dark:border-neutral-800 dark:hover:bg-neutral-700 md:block"
					class:bg-neutral-400={isResizingRight}
					class:dark:bg-neutral-600={isResizingRight}
					onmousedown={handleRightResizeStart}
					role="separator"
					aria-label="Resize right sidebar"
				></div>

				<aside
					class="hidden border-l border-neutral-200 bg-white md:block dark:border-neutral-800 dark:bg-neutral-950"
					style="width: {$uiStore.rightSidebarWidth}px"
				>
					<RightSidebar />
				</aside>
			{/if}

			<!-- Mobile Right Sidebar -->
			<aside
				class="fixed inset-y-0 right-0 z-50 transform border-l border-neutral-200 bg-white transition-transform duration-200 ease-in-out md:hidden dark:border-neutral-800 dark:bg-neutral-950 {$uiStore.mobileRightSidebarOpen
					? 'translate-x-0'
					: 'translate-x-full'}"
				style="width: 280px"
			>
				<RightSidebar />
			</aside>

			<!-- Mobile overlay for right sidebar -->
			{#if $uiStore.mobileRightSidebarOpen}
				<button
					onclick={() => uiStore.closeMobileRightSidebar()}
					class="fixed inset-0 z-40 bg-black/50 md:hidden"
					aria-label="Close info panel"
				></button>
			{/if}
		</div>
	{/if}
</div>
