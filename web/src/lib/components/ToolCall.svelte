<script lang="ts">
	interface Props {
		toolName: string;
		arguments: string;
	}

	let { toolName, arguments: args }: Props = $props();

	function formatArgs(argStr: string): string {
		try {
			const parsed = JSON.parse(argStr);
			return JSON.stringify(parsed, null, 2);
		} catch {
			return argStr;
		}
	}
</script>

<div class="mb-2 rounded-lg border border-blue-200 bg-blue-50 px-3 py-2 dark:border-blue-800 dark:bg-blue-950/30">
	<div class="flex items-center gap-2">
		<svg
			class="h-4 w-4 text-blue-500"
			fill="none"
			stroke="currentColor"
			viewBox="0 0 24 24"
		>
			<path
				stroke-linecap="round"
				stroke-linejoin="round"
				stroke-width="2"
				d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
			/>
			<path
				stroke-linecap="round"
				stroke-linejoin="round"
				stroke-width="2"
				d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
			/>
		</svg>
		<span class="text-sm font-medium text-blue-700 dark:text-blue-300">{toolName}</span>
	</div>
	{#if args && args !== '{}' && args !== 'null'}
		<pre class="mt-1 overflow-x-auto text-xs text-blue-600 dark:text-blue-400"><code>{formatArgs(args)}</code></pre>
	{/if}
</div>
