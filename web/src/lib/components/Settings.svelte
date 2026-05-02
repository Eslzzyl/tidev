<script lang="ts">
	import { uiStore, effectiveTheme, type Theme } from '../stores/ui';
	import { clickOutside } from '../actions/clickOutside';

	const themes: { value: Theme; label: string; icon: string }[] = [
		{ value: 'light', label: 'Light', icon: '☀️' },
		{ value: 'dark', label: 'Dark', icon: '🌙' },
		{ value: 'system', label: 'System', icon: '💻' }
	];

	function handleThemeChange(theme: Theme) {
		uiStore.setTheme(theme);
	}

	function handleClose() {
		uiStore.closeSettings();
	}
</script>

{#if $uiStore.settingsOpen}
	<!-- Overlay -->
	<div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
		<!-- Modal -->
		<div
			use:clickOutside={handleClose}
			class="w-full max-w-md rounded-xl bg-white shadow-2xl dark:bg-neutral-900"
		>
			<!-- Header -->
			<div class="flex items-center justify-between border-b border-neutral-200 px-6 py-4 dark:border-neutral-800">
				<h2 class="text-lg font-semibold text-neutral-900 dark:text-neutral-100">Settings</h2>
				<button
					onclick={handleClose}
					class="rounded p-1 text-neutral-500 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
					aria-label="Close settings"
				>
					<svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
						<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
					</svg>
				</button>
			</div>

			<!-- Content -->
			<div class="p-6">
				<!-- Theme Section -->
				<div>
					<h3 class="mb-3 text-sm font-medium text-neutral-900 dark:text-neutral-100">Appearance</h3>
					<p class="mb-4 text-sm text-neutral-500 dark:text-neutral-400">
						Choose your preferred color theme
					</p>

					<div class="grid grid-cols-3 gap-3">
						{#each themes as theme (theme.value)}
							<button
								onclick={() => handleThemeChange(theme.value)}
								class="flex flex-col items-center gap-2 rounded-lg border p-4 transition-all {theme.value === $uiStore.theme
									? 'border-neutral-900 bg-neutral-50 dark:border-neutral-100 dark:bg-neutral-800'
									: 'border-neutral-200 hover:border-neutral-300 dark:border-neutral-700 dark:hover:border-neutral-600'}"
							>
								<span class="text-2xl">{theme.icon}</span>
								<span class="text-sm font-medium text-neutral-900 dark:text-neutral-100">
									{theme.label}
								</span>
								{#if theme.value === $uiStore.theme}
									<span class="text-xs text-neutral-500 dark:text-neutral-400">Active</span>
								{/if}
							</button>
						{/each}
					</div>
				</div>

				<!-- Current Theme Preview -->
				<div class="mt-6 rounded-lg border border-neutral-200 p-4 dark:border-neutral-800">
					<div class="flex items-center justify-between">
						<span class="text-sm text-neutral-600 dark:text-neutral-400">Current theme</span>
						<span class="rounded bg-neutral-100 px-2 py-1 text-xs font-medium uppercase text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300">
							{$effectiveTheme}
						</span>
					</div>
				</div>
			</div>

			<!-- Footer -->
			<div class="border-t border-neutral-200 px-6 py-4 dark:border-neutral-800">
				<p class="text-center text-xs text-neutral-500 dark:text-neutral-400">
					Settings are saved automatically
				</p>
			</div>
		</div>
	</div>
{/if}
