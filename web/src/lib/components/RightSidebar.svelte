<script lang="ts">
	import { sessionStore } from '../stores/session';
	import { uiStore } from '../stores/ui';
	import type { FileDiff, TodoItem, TokenUsage } from '../api/client';

	// Stats derived from messages
	let stats = $state({ requestCount: 0, totalTokens: 0, inputTokens: 0, outputTokens: 0 });

	$effect(() => {
		const messages = $sessionStore.messages;
		const assistantMessages = messages.filter((m) => m.role === 'assistant');

		// Calculate token usage from message metadata if available
		let totalTokens = 0;
		let inputTokens = 0;
		let outputTokens = 0;

		for (const msg of assistantMessages) {
			const usage = msg.token_usage as TokenUsage | undefined;
			if (usage) {
				totalTokens += usage.total_tokens || 0;
				inputTokens += usage.input_tokens || 0;
				outputTokens += usage.output_tokens || 0;
			}
		}

		stats = {
			requestCount: assistantMessages.length,
			totalTokens,
			inputTokens,
			outputTokens
		};
	});

	// Format workspace path (replace home with ~)
	function formatWorkspace(path: string): string {
		if (!path) return '-';
		const home = window.navigator.platform.includes('Win') ? '%USERPROFILE%' : '~';
		// Simple approximation - in real app would use proper home detection
		if (path.startsWith('/home/') || path.startsWith('/Users/')) {
			const parts = path.split('/');
			if (parts.length >= 3) {
				return home + '/' + parts.slice(3).join('/');
			}
		}
		return path;
	}

	// Format number with commas
	function formatNumber(n: number): string {
		return n.toLocaleString();
	}

	// Get model display info
	let modelInfo = $state<{ name: string; provider: string } | null>(null);

	$effect(() => {
		const session = $sessionStore.currentSession;
		if (!session) {
			modelInfo = null;
		} else {
			modelInfo = {
				name: session.model_display_name,
				provider: session.provider_display_name
			};
		}
	});

	// Parse file diffs from messages
	let fileDiffs = $state<Array<{
		path: string;
		status: 'added' | 'modified' | 'deleted';
		additions: number;
		deletions: number;
	}>>([]);

	$effect(() => {
		const diffs: Array<{
			path: string;
			status: 'added' | 'modified' | 'deleted';
			additions: number;
			deletions: number;
		}> = [];

		for (const msg of $sessionStore.messages) {
			const fileDiffsArr = msg.file_diffs as FileDiff[] | undefined;
			if (fileDiffsArr && Array.isArray(fileDiffsArr)) {
				for (const diff of fileDiffsArr) {
					diffs.push({
						path: diff.path || diff.file_path || 'unknown',
						status: diff.status || 'modified',
						additions: diff.additions || 0,
						deletions: diff.deletions || 0
					});
				}
			}
		}

		// Sort: modified first, then added, then deleted
		const statusOrder = { modified: 0, added: 1, deleted: 2 };
		fileDiffs = diffs.sort((a, b) => statusOrder[a.status] - statusOrder[b.status]);
	});

	// Get status icon for file
	function getFileStatusIcon(status: string): string {
		switch (status) {
			case 'added':
				return '+';
			case 'deleted':
				return '-';
			case 'modified':
			default:
				return '~';
		}
	}

	// Get status color class
	function getStatusColorClass(status: string): string {
		switch (status) {
			case 'added':
				return 'text-green-600 dark:text-green-400';
			case 'deleted':
				return 'text-red-600 dark:text-red-400';
			case 'modified':
			default:
				return 'text-amber-600 dark:text-amber-400';
		}
	}

	// Parse todos from messages
	let todos = $state<Array<{
		content: string;
		status: 'pending' | 'in_progress' | 'completed' | 'cancelled';
		priority: 'low' | 'medium' | 'high';
	}>>([]);

	$effect(() => {
		const items: Array<{
			content: string;
			status: 'pending' | 'in_progress' | 'completed' | 'cancelled';
			priority: 'low' | 'medium' | 'high';
		}> = [];

		for (const msg of $sessionStore.messages) {
			const todosArr = msg.todos as TodoItem[] | undefined;
			if (todosArr && Array.isArray(todosArr)) {
				for (const todo of todosArr) {
					items.push({
						content: todo.content || todo.title || 'Untitled',
						status: todo.status || 'pending',
						priority: todo.priority || 'medium'
					});
				}
			}
		}

		todos = items;
	});

	// Get todo status icon
	function getTodoIcon(status: string): string {
		switch (status) {
			case 'completed':
				return '✔';
			case 'in_progress':
				return '●';
			case 'cancelled':
				return '✗';
			case 'pending':
			default:
				return '○';
		}
	}

	// Check if undo is active (simplified - would check session state)
	let isUndoActive = $state(false);

	$effect(() => {
		const summary = $sessionStore.currentSession?.context_summary;
		isUndoActive = summary?.includes('revert') || false;
	});
</script>

<div class="flex h-full flex-col bg-neutral-50 dark:bg-neutral-900">
	<!-- Header -->
	<div class="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
		<h2 class="text-sm font-semibold text-neutral-900 dark:text-neutral-100">Info</h2>
		<button
			onclick={() => uiStore.toggleSidebar()}
			class="rounded p-1 text-neutral-600 hover:bg-neutral-200 md:hidden dark:text-neutral-400 dark:hover:bg-neutral-800"
			aria-label="Close sidebar"
		>
			<svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
				<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
			</svg>
		</button>
	</div>

	<!-- Content -->
	<div class="flex-1 overflow-y-auto p-4">
		{#if !$sessionStore.currentSession}
			<p class="text-center text-sm text-neutral-500 dark:text-neutral-400">
				Select a session to view details
			</p>
		{:else}
			<div class="space-y-6">
				<!-- Workspace -->
				<div>
					<h3 class="mb-1 text-xs font-medium uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
						Workspace
					</h3>
					<p class="break-all text-sm text-neutral-900 dark:text-neutral-100">
						{formatWorkspace($sessionStore.currentSession.workspace_root)}
					</p>
				</div>

				<!-- Model -->
				<div>
					<h3 class="mb-1 text-xs font-medium uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
						Model
					</h3>
					<p class="text-sm text-neutral-900 dark:text-neutral-100">
						{modelInfo?.name || '-'}
					</p>
					<p class="text-xs text-neutral-500 dark:text-neutral-400">
						{modelInfo?.provider || '-'}
					</p>
				</div>

				<!-- Tokens -->
				<div>
					<h3 class="mb-2 text-xs font-medium uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
						Tokens
					</h3>
					<div class="grid grid-cols-2 gap-2 text-sm">
						<div class="rounded bg-white p-2 dark:bg-neutral-800">
							<span class="text-xs text-neutral-500 dark:text-neutral-400">Total</span>
							<p class="font-medium text-neutral-900 dark:text-neutral-100">
								{formatNumber(stats.totalTokens)}
							</p>
						</div>
						<div class="rounded bg-white p-2 dark:bg-neutral-800">
							<span class="text-xs text-neutral-500 dark:text-neutral-400">Requests</span>
							<p class="font-medium text-neutral-900 dark:text-neutral-100">
								{formatNumber(stats.requestCount)}
							</p>
						</div>
						<div class="rounded bg-white p-2 dark:bg-neutral-800">
							<span class="text-xs text-neutral-500 dark:text-neutral-400">Input</span>
							<p class="font-medium text-neutral-900 dark:text-neutral-100">
								{formatNumber(stats.inputTokens)}
							</p>
						</div>
						<div class="rounded bg-white p-2 dark:bg-neutral-800">
							<span class="text-xs text-neutral-500 dark:text-neutral-400">Output</span>
							<p class="font-medium text-neutral-900 dark:text-neutral-100">
								{formatNumber(stats.outputTokens)}
							</p>
						</div>
					</div>
				</div>

				<!-- Changed Files -->
				{#if fileDiffs.length > 0}
					<div>
						<h3 class="mb-2 text-xs font-medium uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
							Changed Files ({fileDiffs.length})
						</h3>
						<ul class="space-y-1">
							{#each fileDiffs as diff, index (diff.path + index)}
								<li class="flex items-center gap-2 text-xs">
									<span class="font-mono {getStatusColorClass(diff.status)}">
										{getFileStatusIcon(diff.status)}
									</span>
									<span class="flex-1 truncate text-neutral-700 dark:text-neutral-300" title={diff.path}>
										{diff.path}
									</span>
									{#if diff.additions > 0 || diff.deletions > 0}
										<span class="text-neutral-500 dark:text-neutral-400">
											(+{diff.additions}/-{diff.deletions})
										</span>
									{/if}
								</li>
							{/each}
						</ul>
					</div>
				{/if}

				<!-- Todos -->
				{#if todos.length > 0}
					<div>
						<h3 class="mb-2 text-xs font-medium uppercase tracking-wider text-neutral-500 dark:text-neutral-400">
							Todos ({todos.filter((t) => t.status === 'completed').length}/{todos.length})
						</h3>
						<ul class="space-y-1">
							{#each todos as todo, index (todo.content + index)}
								<li class="flex items-start gap-2 text-xs">
									<span class="mt-0.5 flex-shrink-0">
										{getTodoIcon(todo.status)}
									</span>
									<span
										class="flex-1 text-neutral-700 dark:text-neutral-300 {todo.status === 'completed'
											? 'line-through opacity-50'
											: ''}"
									>
										{todo.content}
										{#if todo.priority === 'high'}
											<span class="ml-1 text-amber-500">⚠</span>
										{/if}
									</span>
								</li>
							{/each}
						</ul>
					</div>
				{/if}

				<!-- Undo State -->
				{#if isUndoActive}
					<div class="rounded bg-amber-50 p-3 dark:bg-amber-950">
						<p class="flex items-center gap-2 text-xs text-amber-800 dark:text-amber-200">
							<span>⚠</span>
							<span>Undo active</span>
						</p>
						<p class="mt-1 text-xs text-amber-700 dark:text-amber-300">
							Conversation was reverted. New messages will branch from this point.
						</p>
					</div>
				{/if}
			</div>
		{/if}
	</div>
</div>
