<script lang="ts">
	import type { Message } from '../api/client';
	import MarkdownRenderer from './MarkdownRenderer.svelte';
	import ThinkingBlock from './ThinkingBlock.svelte';
	import ToolResult from './ToolResult.svelte';
	import ToolCall from './ToolCall.svelte';

	interface Props {
		message: Message;
		// Associated tool results for this message (for assistant messages)
		toolResults?: Message[];
	}

	let { message, toolResults = [] }: Props = $props();

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
</script>

<!-- All messages are left-aligned -->
<div class="group flex gap-3 px-4 py-4">
	<!-- Avatar -->
	<div class="flex-shrink-0">
		<div
			class="flex h-8 w-8 items-center justify-center rounded-full text-xs font-medium {message.role ===
			'user'
				? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900'
				: message.role === 'assistant'
					? 'bg-blue-600 text-white dark:bg-blue-500'
					: 'bg-neutral-200 text-neutral-700 dark:bg-neutral-700 dark:text-neutral-300'}"
		>
			{#if message.role === 'user'}
				U
			{:else if message.role === 'assistant'}
				A
			{:else if message.role === 'system'}
				S
			{:else if message.role === 'tool'}
				T
			{:else}
				?
			{/if}
		</div>
	</div>

	<!-- Message Content -->
	<div class="flex max-w-[85%] flex-col items-start">
		<div class="mb-1 flex items-center gap-2">
			<span class="text-xs font-medium text-neutral-500 dark:text-neutral-400">
				{getRoleLabel(message.role)}
			</span>
			<span class="text-xs text-neutral-400 dark:text-neutral-600">
				{new Date(message.created_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
			</span>
		</div>

		<div
			class="w-full rounded-2xl px-4 py-2.5 text-sm leading-relaxed {getRoleColor(message.role)} rounded-tl-sm"
		>
			{#if message.role === 'assistant'}
				<!-- Thinking/Reasoning block -->
				{#if message.reasoning}
					<ThinkingBlock content={message.reasoning} />
				{/if}

				<!-- Main content with markdown -->
				<MarkdownRenderer content={message.content} />

				<!-- Tool calls -->
				{#if message.tool_calls && message.tool_calls.length > 0}
					<div class="mb-2">
						{#each message.tool_calls as toolCall (toolCall.id)}
							<ToolCall toolName={toolCall.name} arguments={toolCall.arguments} />
						{/each}
					</div>
				{/if}

				<!-- Tool results (associated with this message's tool calls) -->
				{#if toolResults.length > 0}
					<div class="mt-3 border-t border-neutral-200 pt-2 dark:border-neutral-700">
						{#each toolResults as result (result.id)}
							<ToolResult
								toolCallId={result.tool_call_id || ''}
								toolName={result.tool_name || 'Unknown'}
								output={result.content}
								isError={result.role === 'error'}
								diff={result.diff}
								filepath={result.filepath}
							/>
						{/each}
					</div>
				{/if}
			{:else if message.role === 'tool'}
				<!-- Standalone tool result message -->
				<ToolResult
					toolCallId={message.tool_call_id || ''}
					toolName={message.tool_name || 'Unknown'}
					output={message.content}
					defaultExpanded={false}
					diff={message.diff}
					filepath={message.filepath}
				/>
			{:else if message.role === 'error'}
				<!-- Error message -->
				<div class="text-red-600 dark:text-red-400">
					<MarkdownRenderer content={message.content} />
				</div>
			{:else}
				<!-- User, System, Shell messages -->
				<p class="whitespace-pre-wrap">{message.content}</p>
			{/if}
		</div>
	</div>
</div>
