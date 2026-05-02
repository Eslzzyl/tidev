<script lang="ts">
	import { marked, type Token } from 'marked';
	import { markedHighlight } from 'marked-highlight';
	import hljs from 'highlight.js';
	import DOMPurify from 'dompurify';
	import { onMount } from 'svelte';

	// Import highlight.js light theme (default)
	import 'highlight.js/styles/github.css';

	interface Props {
		content: string;
	}

	let { content }: Props = $props();

	let htmlContent = $state('');

	const renderer = new marked.Renderer();

	// Custom link renderer to open in new tab
	// Use object destructuring to match new marked API signature
	renderer.link = ({ href, title, tokens }: { href: string; title?: string | null; tokens: Token[] }) => {
		const text = renderer.parser.parseInline(tokens);
		const titleAttr = title ? ` title="${title}"` : '';
		return `<a href="${href}"${titleAttr} target="_blank" rel="noopener noreferrer" class="text-blue-600 hover:text-blue-800 dark:text-blue-400 dark:hover:text-blue-300 underline">${text}</a>`;
	};

	// Configure marked with syntax highlighting
	marked.use(
		markedHighlight({
			emptyLangClass: 'hljs',
			langPrefix: 'hljs language-',
			highlight: (code, lang) => {
				const language = hljs.getLanguage(lang) ? lang : 'plaintext';
				return hljs.highlight(code, { language }).value;
			}
		})
	);

	marked.setOptions({
		renderer,
		breaks: true,
		gfm: true
	});

	onMount(() => {
		updateContent();
	});

	$effect(() => {
		updateContent();
	});

	function updateContent() {
		if (!content) {
			htmlContent = '';
			return;
		}
		const rawHtml = marked.parse(content, { async: false }) as string;
		htmlContent = DOMPurify.sanitize(rawHtml);
	}
</script>

<div class="markdown-body prose prose-sm dark:prose-invert max-w-none">
	{@html htmlContent}
</div>

<style>
	/* Dark theme syntax highlighting overrides */
	:global(.dark .hljs) {
		color: #c9d1d9;
		background: #0d1117;
	}
	:global(.dark .hljs-doctag),
	:global(.dark .hljs-keyword),
	:global(.dark .hljs-meta .hljs-keyword),
	:global(.dark .hljs-template-tag),
	:global(.dark .hljs-template-variable),
	:global(.dark .hljs-type),
	:global(.dark .hljs-variable.language_) {
		color: #ff7b72;
	}
	:global(.dark .hljs-title),
	:global(.dark .hljs-title.class_),
	:global(.dark .hljs-title.class_.inherited__),
	:global(.dark .hljs-title.function_) {
		color: #d2a8ff;
	}
	:global(.dark .hljs-attr),
	:global(.dark .hljs-attribute),
	:global(.dark .hljs-literal),
	:global(.dark .hljs-meta),
	:global(.dark .hljs-number),
	:global(.dark .hljs-operator),
	:global(.dark .hljs-variable),
	:global(.dark .hljs-selector-attr),
	:global(.dark .hljs-selector-class),
	:global(.dark .hljs-selector-id) {
		color: #79c0ff;
	}
	:global(.dark .hljs-regexp),
	:global(.dark .hljs-string),
	:global(.dark .hljs-meta .hljs-string) {
		color: #a5d6ff;
	}
	:global(.dark .hljs-built_in),
	:global(.dark .hljs-symbol) {
		color: #ffa657;
	}
	:global(.dark .hljs-comment),
	:global(.dark .hljs-code),
	:global(.dark .hljs-formula) {
		color: #8b949e;
	}
	:global(.dark .hljs-name),
	:global(.dark .hljs-quote),
	:global(.dark .hljs-selector-tag),
	:global(.dark .hljs-selector-pseudo) {
		color: #7ee787;
	}
	:global(.dark .hljs-subst) {
		color: #c9d1d9;
	}
	:global(.dark .hljs-section) {
		color: #1f6feb;
		font-weight: bold;
	}
	:global(.dark .hljs-bullet) {
		color: #f2cc60;
	}
	:global(.dark .hljs-emphasis) {
		color: #c9d1d9;
		font-style: italic;
	}
	:global(.dark .hljs-strong) {
		color: #c9d1d9;
		font-weight: bold;
	}
	:global(.dark .hljs-addition) {
		color: #aff5b4;
		background-color: #033a16;
	}
	:global(.dark .hljs-deletion) {
		color: #ffdcd7;
		background-color: #67060c;
	}

	:global(.markdown-body pre) {
		margin-top: 0.75rem;
		margin-bottom: 0.75rem;
		overflow-x: auto;
		border-radius: 0.5rem;
		background-color: #171717;
		padding: 1rem;
	}

	:global(.markdown-body pre code) {
		display: block;
		background-color: transparent;
		padding: 0;
		font-size: 0.875rem;
		line-height: 1.625;
		color: #f5f5f5;
	}

	:global(.markdown-body :not(pre) > code) {
		border-radius: 0.25rem;
		background-color: #e5e5e5;
		padding-left: 0.375rem;
		padding-right: 0.375rem;
		padding-top: 0.125rem;
		padding-bottom: 0.125rem;
		font-size: 0.875rem;
		font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
		color: #262626;
	}

	:global(.dark .markdown-body :not(pre) > code) {
		background-color: #404040;
		color: #e5e5e5;
	}

	:global(.markdown-body p) {
		margin-top: 0.5rem;
		margin-bottom: 0.5rem;
		line-height: 1.625;
	}

	:global(.markdown-body ul, .markdown-body ol) {
		margin-top: 0.5rem;
		margin-bottom: 0.5rem;
		margin-left: 1.25rem;
		list-style-type: disc;
	}

	:global(.markdown-body ol) {
		list-style-type: decimal;
	}

	:global(.markdown-body li) {
		margin-top: 0.25rem;
		margin-bottom: 0.25rem;
	}

	:global(.markdown-body h1, .markdown-body h2, .markdown-body h3, .markdown-body h4) {
		margin-top: 0.75rem;
		margin-bottom: 0.75rem;
		font-weight: 600;
	}

	:global(.markdown-body h1) {
		font-size: 1.25rem;
	}

	:global(.markdown-body h2) {
		font-size: 1.125rem;
	}

	:global(.markdown-body h3) {
		font-size: 1rem;
	}

	:global(.markdown-body blockquote) {
		margin-top: 0.5rem;
		margin-bottom: 0.5rem;
		border-left-width: 4px;
		border-color: #d4d4d4;
		padding-left: 1rem;
		font-style: italic;
		color: #525252;
	}

	:global(.dark .markdown-body blockquote) {
		border-color: #525252;
		color: #a3a3a3;
	}

	:global(.markdown-body table) {
		margin-top: 0.75rem;
		margin-bottom: 0.75rem;
		width: 100%;
		border-collapse: collapse;
		font-size: 0.875rem;
	}

	:global(.markdown-body th, .markdown-body td) {
		border-width: 1px;
		border-color: #d4d4d4;
		padding-left: 0.75rem;
		padding-right: 0.75rem;
		padding-top: 0.5rem;
		padding-bottom: 0.5rem;
	}

	:global(.dark .markdown-body th, .dark .markdown-body td) {
		border-color: #525252;
	}

	:global(.markdown-body th) {
		background-color: #f5f5f5;
		font-weight: 600;
	}

	:global(.dark .markdown-body th) {
		background-color: #262626;
	}

	:global(.markdown-body hr) {
		margin-top: 1rem;
		margin-bottom: 1rem;
		border-color: #d4d4d4;
	}

	:global(.dark .markdown-body hr) {
		border-color: #525252;
	}
</style>
