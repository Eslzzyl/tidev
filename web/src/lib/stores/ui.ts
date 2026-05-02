import { writable, derived } from 'svelte/store';

export type Theme = 'light' | 'dark' | 'system';

export interface UIState {
	sidebarOpen: boolean;
	settingsOpen: boolean;
	mobileMenuOpen: boolean;
	theme: Theme;
	inputValue: string;
	isLoading: boolean;
	isStreaming: boolean;
	connectionStatus: 'connected' | 'disconnected' | 'connecting';
}

function createUIStore() {
	const { subscribe, set, update } = writable<UIState>({
		sidebarOpen: true,
		settingsOpen: false,
		mobileMenuOpen: false,
		theme: 'system',
		inputValue: '',
		isLoading: false,
		isStreaming: false,
		connectionStatus: 'disconnected'
	});

	// Load theme from localStorage on init
	if (typeof localStorage !== 'undefined') {
		const savedTheme = localStorage.getItem('theme') as Theme | null;
		if (savedTheme) {
			update((s) => ({ ...s, theme: savedTheme }));
		}
	}

	return {
		subscribe,
		toggleSidebar: () => update((s) => ({ ...s, sidebarOpen: !s.sidebarOpen })),
		openSidebar: () => update((s) => ({ ...s, sidebarOpen: true })),
		closeSidebar: () => update((s) => ({ ...s, sidebarOpen: false })),
		toggleSettings: () => update((s) => ({ ...s, settingsOpen: !s.settingsOpen })),
		closeSettings: () => update((s) => ({ ...s, settingsOpen: false })),
		toggleMobileMenu: () => update((s) => ({ ...s, mobileMenuOpen: !s.mobileMenuOpen })),
		closeMobileMenu: () => update((s) => ({ ...s, mobileMenuOpen: false })),
		setTheme: (theme: Theme) => {
			if (typeof localStorage !== 'undefined') {
				localStorage.setItem('theme', theme);
			}
			update((s) => ({ ...s, theme }));
		},
		setInputValue: (value: string) => update((s) => ({ ...s, inputValue: value })),
		setLoading: (isLoading: boolean) => update((s) => ({ ...s, isLoading })),
		setStreaming: (isStreaming: boolean) => update((s) => ({ ...s, isStreaming })),
		setConnectionStatus: (status: UIState['connectionStatus']) =>
			update((s) => ({ ...s, connectionStatus: status }))
	};
}

export const uiStore = createUIStore();

// Derived stores
export const isSidebarVisible = derived(
	uiStore,
	($ui) => $ui.sidebarOpen
);

export const effectiveTheme = derived(uiStore, ($ui) => {
	if ($ui.theme === 'system') {
		if (typeof window !== 'undefined') {
			return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
		}
		return 'light';
	}
	return $ui.theme;
});
