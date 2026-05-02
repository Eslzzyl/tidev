<script lang="ts">
	import { untrack } from 'svelte';
	import MarkdownRenderer from './MarkdownRenderer.svelte';
	import DiffRenderer from './DiffRenderer.svelte';

	interface Props {
		toolCallId: string;
		toolName: string;
		output: string;
		isError?: boolean;
		exitCode?: number | null;
		defaultExpanded?: boolean;
		// Diff content for write/edit tool results
		diff?: string;
		filepath?: string;
	}

	let {
		toolName,
		output,
		isError = false,
		exitCode = null,
		defaultExpanded = false,
		diff = '',
		filepath = ''
	}: Props = $props();

	// Use untrack to indicate we only want the initial value
	let expanded = $state(untrack(() => defaultExpanded));

	function toggle() {
		expanded = !expanded;
	}

	function getOutputPreview(text: string): string {
		const maxLen = 80;
		const firstLine = text.split('\n')[0] || '';
		if (firstLine.length <= maxLen) return firstLine || '(no output)';
		return firstLine.slice(0, maxLen) + '...';
	}

	function getStatusIcon(): string {
		if (isError) return 'error';
		if (exitCode !== null && exitCode !== 0) return 'error';
		return 'success';
	}
</script>

<div class="mb-2 overflow-hidden rounded-lg border border-neutral-200 bg-white dark:border-neutral-700 dark:bg-neutral-800">
	<button
		onclick={toggle}
		class="flex w-full items-center justify-between px-3 py-2 text-left transition-colors hover:bg-neutral-50 dark:hover:bg-neutral-700/50"
	>
		<div class="flex items-center gap-2 overflow-hidden">
			{#if getStatusIcon() === 'success'}
				<svg
					class="h-4 w-4 flex-shrink-0 text-green-500"
					fill="none"
					stroke="currentColor"
					viewBox="0 0 24 24"
				>
					<path
						stroke-linecap="round"
						stroke-linejoin="round"
						stroke-width="2"
						d="M5 13l4 4L19 7"
					/>
				</svg>
			{:else}
				<svg
					class="h-4 w-4 flex-shrink-0 text-red-500"
					fill="none"
					stroke="currentColor"
					viewBox="0 0 24 24"
				>
					<path
						stroke-linecap="round"
						stroke-linejoin="round"
						stroke-width="2"
						d="M6 18L18 6M6 6l12 12"
					/>
				</svg>
			{/if}
			<span class="text-sm font-medium text-neutral-700 dark:text-neutral-300">
				{toolName}
			</span>
			<span class="truncate text-sm text-neutral-500 dark:text-neutral-400">
				{expanded ? '' : getOutputPreview(output)}
			</span>
		</div>
		<svg
			class="ml-2 h-4 w-4 flex-shrink-0 transform text-neutral-400 transition-transform {expanded
				? 'rotate-180'
				: ''}"
			fill="none"
			stroke="currentColor"
			viewBox="0 0 24 24"
		>
			<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" />
		</svg>
	</button>

	{#if expanded}
		<div class="border-t border-neutral-200 dark:border-neutral-700">
			<div class="max-h-96 overflow-auto bg-neutral-50 p-3 dark:bg-neutral-900/50">
				{#if diff}
					<DiffRenderer {diff} {filepath} />
				{:else}
					<MarkdownRenderer content={output} />
				{/if}
			</div>
			{#if exitCode !== null}
				<div class="border-t border-neutral-200 bg-neutral-100 px-3 py-1 text-xs dark:border-neutral-700 dark:bg-neutral-800">
					<span class="text-neutral-500 dark:text-neutral-400">Exit code: {exitCode}</span>
				</div>
			{/if}
		</div>
	{/if}
</div>
