<script lang="ts">
	import { parsePatch } from 'diff';
	import hljs from 'highlight.js';
	import { onMount } from 'svelte';

	interface Props {
		diff: string;
		filepath: string;
	}

	let { diff, filepath }: Props = $props();

	const WIDE_LAYOUT_THRESHOLD = 768; // px

	let containerEl: HTMLDivElement | undefined;
	let containerWidth = $state(0);
	let isWide = $state(false);

	// Detect language from file extension
	function detectLanguage(fp: string): string {
		const ext = fp.split('.').pop()?.toLowerCase() || '';
		const langMap: Record<string, string> = {
			rs: 'rust',
			ts: 'typescript',
			tsx: 'tsx',
			js: 'javascript',
			jsx: 'jsx',
			py: 'python',
			go: 'go',
			rb: 'ruby',
			java: 'java',
			kt: 'kotlin',
			scala: 'scala',
			swift: 'swift',
			c: 'c',
			h: 'c',
			cpp: 'cpp',
			hpp: 'cpp',
			cc: 'cpp',
			hh: 'cpp',
			cxx: 'cpp',
			cs: 'csharp',
			php: 'php',
			html: 'html',
			css: 'css',
			scss: 'scss',
			sass: 'sass',
			less: 'less',
			sql: 'sql',
			sh: 'bash',
			bash: 'bash',
			zsh: 'bash',
			yaml: 'yaml',
			yml: 'yaml',
			toml: 'toml',
			json: 'json',
			xml: 'xml',
			md: 'markdown',
			mdx: 'markdown',
			svelte: 'svelte',
			vue: 'vue',
			lua: 'lua',
			dart: 'dart',
			r: 'r',
			zig: 'zig',
			nim: 'nim',
		};
		return langMap[ext] || '';
	}

	const language = $derived(detectLanguage(filepath));

	// Highlight a single line of code
	function highlightLine(line: string): string {
		if (!language) {
			// Escape HTML
			return line
				.replace(/&/g, '&amp;')
				.replace(/</g, '&lt;')
				.replace(/>/g, '&gt;');
		}
		// highlight.js works best on full blocks, but for diffs we highlight snippet-style
		try {
			const result = hljs.highlight(line, { language, ignoreIllegals: true });
			return result.value;
		} catch {
			return line
				.replace(/&/g, '&amp;')
				.replace(/</g, '&lt;')
				.replace(/>/g, '&gt;');
		}
	}

	// Escape HTML without highlighting
	function escapeHtml(text: string): string {
		return text
			.replace(/&/g, '&amp;')
			.replace(/</g, '&lt;')
			.replace(/>/g, '&gt;');
	}

	interface DiffLine {
		type: 'context' | 'add' | 'del';
		content: string;
		oldLineNum: number | null;
		newLineNum: number | null;
	}

	interface AlignedRow {
		left: { type: 'context' | 'del' | 'empty'; content: string; lineNum: number | null } | null;
		right: { type: 'context' | 'add' | 'empty'; content: string; lineNum: number | null } | null;
		rowKind: 'context' | 'removed' | 'added' | 'modified';
	}

	interface ParsedFileDiff {
		oldPath: string;
		newPath: string;
		status: 'added' | 'modified' | 'deleted';
		alignedRows: AlignedRow[];
	}

	// Parse the unified diff and produce aligned rows for side-by-side view
	function parseAndAlign(diffText: string): ParsedFileDiff[] {
		const patches = parsePatch(diffText);
		return patches.map((patch) => {
			const oldPath = patch.oldFileName?.replace(/^[ab]\//, '') || filepath;
			const newPath = patch.newFileName?.replace(/^[ab]\//, '') || filepath;

			const isAdded = oldPath === '/dev/null';
			const isDeleted = newPath === '/dev/null';
			const status: 'added' | 'modified' | 'deleted' = isAdded
				? 'added'
				: isDeleted
					? 'deleted'
					: 'modified';

			const alignedRows: AlignedRow[] = [];

			for (const hunk of patch.hunks) {
				let oldLineNum = hunk.oldStart;
				let newLineNum = hunk.newStart;

				for (const line of hunk.lines) {
					const ch = line[0];
					const content = line.slice(1);

					if (ch === ' ') {
						// Context: appears on both sides
						alignedRows.push({
							left: { type: 'context', content, lineNum: oldLineNum },
							right: { type: 'context', content, lineNum: newLineNum },
							rowKind: 'context',
						});
						oldLineNum++;
						newLineNum++;
					} else if (ch === '-') {
						// Deletion: left only
						alignedRows.push({
							left: { type: 'del', content, lineNum: oldLineNum },
							right: null,
							rowKind: 'removed',
						});
						oldLineNum++;
					} else if (ch === '+') {
						// Addition: right only
						// Check if we can pair with previous unmatched deletion
						const lastRow =
							alignedRows.length > 0
								? alignedRows[alignedRows.length - 1]
								: null;
						if (
							lastRow &&
							lastRow.rowKind === 'removed' &&
							lastRow.right === null
						) {
							// Pair with previous deletion to show as "modified"
							lastRow.right = {
								type: 'add',
								content,
								lineNum: newLineNum,
							};
							lastRow.rowKind = 'modified';
						} else {
							alignedRows.push({
								left: null,
								right: { type: 'add', content, lineNum: newLineNum },
								rowKind: 'added',
							});
						}
						newLineNum++;
					}
					// '\' lines (no newline at end) are informational, skip
				}
			}

			return { oldPath, newPath, status, alignedRows };
		});
	}

	const parsedDiffs = $derived(parseAndAlign(diff));

	// For single-column (narrow) rendering, produce a flat list of diff rows
	interface FlatRow {
		type: 'context' | 'add' | 'del' | 'hunk-header';
		content: string;
		oldLineNum: number | null;
		newLineNum: number | null;
	}

	const flatRows = $derived.by(() => {
		const rows: FlatRow[] = [];
		for (const fileDiff of parsedDiffs) {
			// File header
			const statusLabel =
				fileDiff.status === 'added'
					? 'Added'
					: fileDiff.status === 'deleted'
						? 'Deleted'
						: 'Modified';
			rows.push({
				type: 'hunk-header',
				content: `${fileDiff.oldPath} → ${fileDiff.newPath} (${statusLabel})`,
				oldLineNum: null,
				newLineNum: null,
			});

			for (const row of fileDiff.alignedRows) {
				if (row.rowKind === 'context') {
					rows.push({
						type: 'context',
						content: row.left!.content,
						oldLineNum: row.left!.lineNum,
						newLineNum: row.right!.lineNum,
					});
				} else if (row.rowKind === 'removed') {
					rows.push({
						type: 'del',
						content: row.left!.content,
						oldLineNum: row.left!.lineNum,
						newLineNum: null,
					});
				} else if (row.rowKind === 'added') {
					rows.push({
						type: 'add',
						content: row.right!.content,
						oldLineNum: null,
						newLineNum: row.right!.lineNum,
					});
				} else if (row.rowKind === 'modified') {
					rows.push({
						type: 'del',
						content: row.left!.content,
						oldLineNum: row.left!.lineNum,
						newLineNum: null,
					});
					rows.push({
						type: 'add',
						content: row.right!.content,
						oldLineNum: null,
						newLineNum: row.right!.lineNum,
					});
				}
			}
		}
		return rows;
	});

	// ResizeObserver for responsive layout
	onMount(() => {
		if (!containerEl) return;
		const observer = new ResizeObserver((entries) => {
			for (const entry of entries) {
				containerWidth = entry.contentRect.width;
				isWide = containerWidth >= WIDE_LAYOUT_THRESHOLD;
			}
		});
		observer.observe(containerEl);
		return () => observer.disconnect();
	});

	function statusIcon(status: string): string {
		switch (status) {
			case 'added':
				return '⊕';
			case 'deleted':
				return '⊖';
			case 'modified':
				return '✎';
			default:
				return '·';
		}
	}

	function statusColor(status: string): string {
		switch (status) {
			case 'added':
				return 'text-green-600 dark:text-green-400';
			case 'deleted':
				return 'text-red-600 dark:text-red-400';
			case 'modified':
				return 'text-amber-600 dark:text-amber-400';
			default:
				return '';
		}
	}
</script>

<div bind:this={containerEl} class="diff-container overflow-x-auto rounded-lg border border-neutral-200 dark:border-neutral-700">
	{#if parsedDiffs.length === 0}
		<div class="p-4 text-sm text-neutral-500 dark:text-neutral-400">
			No diff content to display.
		</div>
	{:else if isWide}
		<!-- Wide layout: side-by-side (two-column) -->
		{#each parsedDiffs as fileDiff, fileIdx (fileDiff.oldPath + fileDiff.newPath + fileIdx)}
			<!-- File header -->
			<div class="diff-file-header flex items-center gap-2 border-b border-neutral-200 bg-neutral-50 px-4 py-2 text-sm font-mono dark:border-neutral-700 dark:bg-neutral-800/50">
				<span class={statusColor(fileDiff.status)}>{statusIcon(fileDiff.status)}</span>
				<span class="text-neutral-700 dark:text-neutral-300">{fileDiff.oldPath}</span>
				{#if fileDiff.oldPath !== fileDiff.newPath}
					<span class="text-neutral-400">→</span>
					<span class="text-neutral-700 dark:text-neutral-300">{fileDiff.newPath}</span>
				{/if}
			</div>

			<!-- Side-by-side diff table -->
			<div class="diff-two-column">
				{#each fileDiff.alignedRows as row, rowIdx (rowIdx)}
					{@const leftCell = row.left}
					{@const rightCell = row.right}
					<div
						class="diff-row"
						class:diff-row-removed={row.rowKind === 'removed'}
						class:diff-row-added={row.rowKind === 'added' || row.rowKind === 'modified'
						}
					>
						<!-- Left cell -->
						<div
							class="diff-cell diff-cell-left {leftCell?.type === 'del'
								? 'bg-red-50 dark:bg-red-950/40'
								: ''}"
						>
							<span class="diff-line-num">{leftCell?.lineNum ?? ''}</span>
							<span class="diff-line-prefix">
								{leftCell?.type === 'del'
									? '−'
									: ' '}
							</span>
							<span class="diff-line-content">
								{@html leftCell ? highlightLine(leftCell.content) : ''}
							</span>
						</div>

						<!-- Separator -->
						<div class="diff-separator">│</div>

						<!-- Right cell -->
						<div
							class="diff-cell diff-cell-right {rightCell?.type === 'add'
								? 'bg-green-50 dark:bg-green-950/40'
								: ''}"
						>
							<span class="diff-line-num">{rightCell?.lineNum ?? ''}</span>
							<span class="diff-line-prefix">
								{rightCell?.type === 'add'
									? '+'
									: ' '}
							</span>
							<span class="diff-line-content">
								{@html rightCell ? highlightLine(rightCell.content) : ''}
							</span>
						</div>
					</div>
				{/each}
			</div>
		{/each}
	{:else}
		<!-- Narrow layout: unified (single-column) -->
		{#each parsedDiffs as fileDiff, fileIdx (fileDiff.oldPath + fileDiff.newPath + fileIdx)}
			<!-- File header -->
			<div class="diff-file-header flex items-center gap-2 border-b border-neutral-200 bg-neutral-50 px-4 py-2 text-sm font-mono dark:border-neutral-700 dark:bg-neutral-800/50">
				<span class={statusColor(fileDiff.status)}>{statusIcon(fileDiff.status)}</span>
				<span class="text-neutral-700 dark:text-neutral-300">{fileDiff.oldPath}</span>
				{#if fileDiff.oldPath !== fileDiff.newPath}
					<span class="text-neutral-400">→</span>
					<span class="text-neutral-700 dark:text-neutral-300">{fileDiff.newPath}</span>
				{/if}
			</div>

			<!-- Unified diff -->
			<div class="diff-unified">
				{#each fileDiff.alignedRows as row, rowIdx (rowIdx)}
					{@const leftCell = row.left}
					{@const rightCell = row.right}
					{#if row.rowKind === 'context'}
						<div class="diff-row-unified flex">
							<span class="diff-line-num shrink-0 select-none text-right tabular-nums text-neutral-400 dark:text-neutral-500">{leftCell?.lineNum ?? ''}</span>
							<span class="diff-line-num shrink-0 select-none text-right tabular-nums text-neutral-400 dark:text-neutral-500">{rightCell?.lineNum ?? ''}</span>
							<span class="diff-line-prefix shrink-0 w-4 select-none text-neutral-300 dark:text-neutral-600"> </span>
							<span class="diff-line-content min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-pre font-mono text-sm leading-relaxed">
								{@html leftCell ? highlightLine(leftCell.content) : ''}
							</span>
						</div>
					{:else if row.rowKind === 'removed'}
						<div class="diff-row-unified flex bg-red-50 dark:bg-red-950/40">
							<span class="diff-line-num shrink-0 select-none text-right tabular-nums text-red-400 dark:text-red-500">{leftCell?.lineNum ?? ''}</span>
							<span class="diff-line-num shrink-0 select-none text-right tabular-nums text-neutral-400 dark:text-neutral-500"> </span>
							<span class="diff-line-prefix shrink-0 w-4 select-none text-red-500">−</span>
							<span class="diff-line-content min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-pre font-mono text-sm leading-relaxed">
								{@html leftCell ? highlightLine(leftCell.content) : ''}
							</span>
						</div>
					{:else if row.rowKind === 'added'}
						<div class="diff-row-unified flex bg-green-50 dark:bg-green-950/40">
							<span class="diff-line-num shrink-0 select-none text-right tabular-nums text-neutral-400 dark:text-neutral-500"> </span>
							<span class="diff-line-num shrink-0 select-none text-right tabular-nums text-green-400 dark:text-green-500">{rightCell?.lineNum ?? ''}</span>
							<span class="diff-line-prefix shrink-0 w-4 select-none text-green-600 dark:text-green-400">+</span>
							<span class="diff-line-content min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-pre font-mono text-sm leading-relaxed">
								{@html rightCell ? highlightLine(rightCell.content) : ''}
							</span>
						</div>
					{:else if row.rowKind === 'modified'}
						<!-- Show modified as deletion + addition pair in unified view -->
						<div class="diff-row-unified flex bg-red-50 dark:bg-red-950/40">
							<span class="diff-line-num shrink-0 select-none text-right tabular-nums text-red-400 dark:text-red-500">{leftCell?.lineNum ?? ''}</span>
							<span class="diff-line-num shrink-0 select-none text-right tabular-nums text-neutral-400 dark:text-neutral-500"> </span>
							<span class="diff-line-prefix shrink-0 w-4 select-none text-red-500">−</span>
							<span class="diff-line-content min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-pre font-mono text-sm leading-relaxed">
								{@html leftCell ? highlightLine(leftCell.content) : ''}
							</span>
						</div>
						<div class="diff-row-unified flex bg-green-50 dark:bg-green-950/40">
							<span class="diff-line-num shrink-0 select-none text-right tabular-nums text-neutral-400 dark:text-neutral-500"> </span>
							<span class="diff-line-num shrink-0 select-none text-right tabular-nums text-green-400 dark:text-green-500">{rightCell?.lineNum ?? ''}</span>
							<span class="diff-line-prefix shrink-0 w-4 select-none text-green-600 dark:text-green-400">+</span>
							<span class="diff-line-content min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-pre font-mono text-sm leading-relaxed">
								{@html rightCell ? highlightLine(rightCell.content) : ''}
							</span>
						</div>
					{/if}
				{/each}
			</div>
		{/each}
	{/if}
</div>

<style>
	.diff-cell {
		padding: 0 0.25rem;
		min-height: 1.625em;
		display: table-cell;
		vertical-align: top;
		white-space: pre;
		font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
		font-size: 0.8125rem;
	}

	.diff-separator {
		font-size: 0.75rem;
		line-height: 1.625;
		user-select: none;
		display: table-cell;
		vertical-align: top;
		width: 1.25rem;
		text-align: center;
		color: #a3a3a3;
		border-left: 1px solid #e5e5e5;
		border-right: 1px solid #e5e5e5;
	}

	:global(.dark) .diff-separator {
		border-left-color: #404040;
		border-right-color: #404040;
		color: #737373;
	}

	.diff-row-unified {
		min-height: 1.625em;
	}

	.diff-line-num {
		font-variant-numeric: tabular-nums;
		user-select: none;
		color: #a3a3a3;
		display: inline-block;
		min-width: 4ch;
		text-align: right;
		padding-right: 0.5ch;
		font-size: 0.75rem;
		line-height: 1.625;
	}

	:global(.dark) .diff-line-num {
		color: #737373;
	}

	.diff-line-prefix {
		width: 1.5ch;
		text-align: center;
		font-size: 0.75rem;
		line-height: 1.625;
		user-select: none;
		display: inline-block;
	}

	.diff-line-content {
		padding-left: 0.5ch;
		line-height: 1.625;
		font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
		font-size: 0.8125rem;
		white-space: pre-wrap;
		word-break: break-all;
	}

	/* Two-column (side-by-side) table layout */
	.diff-two-column {
		overflow-x: auto;
		display: table;
		width: 100%;
	}

	.diff-row {
		display: table-row;
		width: 100%;
	}
</style>
