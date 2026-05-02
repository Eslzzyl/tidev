<script lang="ts">
	import type { Message } from '../api/client';

	interface Props {
		message: Message;
	}

	let { message }: Props = $props();

	function getRoleColor(role: string): string {
		switch (role) {
			case 'user':
				return 'bg-neutral-100 text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100';
			case 'assistant':
				return 'bg-white text-neutral-900 dark:bg-neutral-900 dark:text-neutral-100';
			case 'system':
				return 'bg-amber-50 text-amber-900 dark:bg-amber-950 dark:text-amber-100';
			case 'tool':
				return 'bg-blue-50 text-blue-900 dark:bg-blue-950 dark:text-blue-100';
			case 'error':
				return 'bg-red-50 text-red-900 dark:bg-red-950 dark:text-red-100';
			default:
				return 'bg-neutral-100 text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100';
		}
	}

	function getRoleLabel(role: string): string {
		switch (role) {
			case 'user':
				return 'You';
			case 'assistant':
				return 'Assistant';
			case 'system':
				return 'System';
			case 'tool':
				return 'Tool';
			case 'error':
				return 'Error';
			default:
				return role;
		}
	}

	function formatContent(content: string): string {
		// Simple formatting for code blocks
		return content;
	}
</script>

<div class="group flex gap-3 px-4 py-4 {message.role === 'user' ? 'flex-row-reverse' : ''}">
	<!-- Avatar -->
	<div class="flex-shrink-0">
		<div
			class="flex h-8 w-8 items-center justify-center rounded-full text-xs font-medium {message.role ===
			'user'
				? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900'
				: 'bg-neutral-200 text-neutral-700 dark:bg-neutral-700 dark:text-neutral-300'}"
		>
			{message.role === 'user' ? 'U' : 'A'}
		</div>
	</div>

	<!-- Message Content -->
	<div class="flex max-w-[85%] flex-col {message.role === 'user' ? 'items-end' : 'items-start'}">
		<div class="mb-1 flex items-center gap-2">
			<span class="text-xs font-medium text-neutral-500 dark:text-neutral-400">
				{getRoleLabel(message.role)}
			</span>
			<span class="text-xs text-neutral-400 dark:text-neutral-600">
				{new Date(message.created_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
			</span>
		</div>

		<div
			class="rounded-2xl px-4 py-2.5 text-sm leading-relaxed {getRoleColor(message.role)} {message.role ===
			'user'
				? 'rounded-tr-sm'
				: 'rounded-tl-sm'}"
		>
			{#if message.role === 'assistant'}
				<!-- For assistant messages, render with markdown-like formatting -->
				<div class="prose prose-sm dark:prose-invert max-w-none">
					{#each formatContent(message.content).split('\n') as line}
						{#if line.startsWith('```')}
							<pre class="mt-2 overflow-x-auto rounded bg-neutral-900 p-3 text-neutral-100"><code>{line.replace(/```/g, '')}</code></pre>
						{:else if line.startsWith('`') && line.endsWith('`')}
							<code class="rounded bg-neutral-200 px-1 py-0.5 text-xs dark:bg-neutral-700">{line.slice(1, -1)}</code>
						{:else}
							<p class="mb-1 last:mb-0">{line}</p>
						{/if}
					{/each}
				</div>
			{:else}
				<p class="whitespace-pre-wrap">{message.content}</p>
			{/if}
		</div>
	</div>
</div>
