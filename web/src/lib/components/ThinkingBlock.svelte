<script lang="ts">
	import { untrack } from 'svelte';

	interface Props {
		content: string;
		tokenCount?: number;
		defaultExpanded?: boolean;
	}

	let { content, tokenCount, defaultExpanded = false }: Props = $props();

	// Use untrack to indicate we only want the initial value
	let expanded = $state(untrack(() => defaultExpanded));

	function toggle() {
		expanded = !expanded;
	}

	function getPreview(text: string): string {
		const maxLen = 100;
		if (text.length <= maxLen) return text;
		return text.slice(0, maxLen) + '...';
	}
</script>

<div class="mb-3 overflow-hidden rounded-lg border border-amber-200 bg-amber-50 dark:border-amber-800 dark:bg-amber-950/50">
	<button
		onclick={toggle}
		class="flex w-full items-center justify-between px-4 py-2 text-left transition-colors hover:bg-amber-100 dark:hover:bg-amber-900/50"
	>
		<div class="flex items-center gap-2">
			<svg
				class="h-4 w-4 text-amber-600 dark:text-amber-400"
				fill="none"
				stroke="currentColor"
				viewBox="0 0 24 24"
			>
				<path
					stroke-linecap="round"
					stroke-linejoin="round"
					stroke-width="2"
					d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
				/>
			</svg>
			<span class="text-sm font-medium text-amber-800 dark:text-amber-200">
				Thinking{tokenCount ? ` (${tokenCount.toLocaleString()} tokens)` : ''}
			</span>
		</div>
		<svg
			class="h-4 w-4 transform text-amber-600 transition-transform dark:text-amber-400 {expanded
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
		<div class="border-t border-amber-200 px-4 py-3 dark:border-amber-800">
			<pre class="whitespace-pre-wrap font-mono text-sm leading-relaxed text-amber-900 dark:text-amber-100">{content}</pre>
		</div>
	{:else}
		<div class="border-t border-amber-200 px-4 py-2 dark:border-amber-800">
			<p class="truncate text-sm text-amber-700/70 dark:text-amber-300/70">
				{getPreview(content)}
			</p>
		</div>
	{/if}
</div>
