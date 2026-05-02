import { writable, derived } from 'svelte/store';

export type Theme = 'light' | 'dark' | 'system';

export interface UIState {
	sidebarOpen: boolean;
	rightSidebarOpen: boolean;
	settingsOpen: boolean;
	mobileMenuOpen: boolean;
	mobileRightSidebarOpen: boolean;
	theme: Theme;
	inputValue: string;
	isLoading: boolean;
	isStreaming: boolean;
	connectionStatus: 'connected' | 'disconnected' | 'connecting';
	// Sidebar widths (in pixels)
	leftSidebarWidth: number;
	rightSidebarWidth: number;
}

const DEFAULT_LEFT_SIDEBAR_WIDTH = 256; // 64 * 4 = 256px (w-64)
const DEFAULT_RIGHT_SIDEBAR_WIDTH = 280;
const MIN_SIDEBAR_WIDTH = 180;
const MAX_SIDEBAR_WIDTH = 500;

function createUIStore() {
	// Load saved values from localStorage
	const savedLeftWidth = typeof localStorage !== 'undefined'
		? parseInt(localStorage.getItem('leftSidebarWidth') || '', 10)
		: NaN;
	const savedRightWidth = typeof localStorage !== 'undefined'
		? parseInt(localStorage.getItem('rightSidebarWidth') || '', 10)
		: NaN;
	const savedTheme = typeof localStorage !== 'undefined'
		? localStorage.getItem('theme') as Theme | null
		: null;
	const savedRightSidebarOpen = typeof localStorage !== 'undefined'
		? localStorage.getItem('rightSidebarOpen') === 'true'
		: true;

	const { subscribe, update } = writable<UIState>({
		sidebarOpen: true,
		rightSidebarOpen: savedRightSidebarOpen,
		settingsOpen: false,
		mobileMenuOpen: false,
		mobileRightSidebarOpen: false,
		theme: savedTheme || 'system',
		inputValue: '',
		isLoading: false,
		isStreaming: false,
		connectionStatus: 'disconnected',
		leftSidebarWidth: isNaN(savedLeftWidth) ? DEFAULT_LEFT_SIDEBAR_WIDTH : savedLeftWidth,
		rightSidebarWidth: isNaN(savedRightWidth) ? DEFAULT_RIGHT_SIDEBAR_WIDTH : savedRightWidth
	});

	return {
		subscribe,
		toggleSidebar: () => update((s) => ({ ...s, sidebarOpen: !s.sidebarOpen })),
		openSidebar: () => update((s) => ({ ...s, sidebarOpen: true })),
		closeSidebar: () => update((s) => ({ ...s, sidebarOpen: false })),
		toggleRightSidebar: () => update((s) => {
			const newOpen = !s.rightSidebarOpen;
			if (typeof localStorage !== 'undefined') {
				localStorage.setItem('rightSidebarOpen', String(newOpen));
			}
			return { ...s, rightSidebarOpen: newOpen };
		}),
		openRightSidebar: () => update((s) => {
			if (typeof localStorage !== 'undefined') {
				localStorage.setItem('rightSidebarOpen', 'true');
			}
			return { ...s, rightSidebarOpen: true };
		}),
		closeRightSidebar: () => update((s) => {
			if (typeof localStorage !== 'undefined') {
				localStorage.setItem('rightSidebarOpen', 'false');
			}
			return { ...s, rightSidebarOpen: false };
		}),
		toggleSettings: () => update((s) => ({ ...s, settingsOpen: !s.settingsOpen })),
		closeSettings: () => update((s) => ({ ...s, settingsOpen: false })),
		toggleMobileMenu: () => update((s) => ({ ...s, mobileMenuOpen: !s.mobileMenuOpen })),
		closeMobileMenu: () => update((s) => ({ ...s, mobileMenuOpen: false })),
		toggleMobileRightSidebar: () => update((s) => ({ ...s, mobileRightSidebarOpen: !s.mobileRightSidebarOpen })),
		closeMobileRightSidebar: () => update((s) => ({ ...s, mobileRightSidebarOpen: false })),
		setTheme: (theme: Theme) => {
			if (typeof localStorage !== 'undefined') {
				localStorage.setItem('theme', theme);
			}
			update((s) => ({ ...s, theme }));
		},
		setLeftSidebarWidth: (width: number) => {
			const clampedWidth = Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, width));
			if (typeof localStorage !== 'undefined') {
				localStorage.setItem('leftSidebarWidth', String(clampedWidth));
			}
			update((s) => ({ ...s, leftSidebarWidth: clampedWidth }));
		},
		setRightSidebarWidth: (width: number) => {
			const clampedWidth = Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, width));
			if (typeof localStorage !== 'undefined') {
				localStorage.setItem('rightSidebarWidth', String(clampedWidth));
			}
			update((s) => ({ ...s, rightSidebarWidth: clampedWidth }));
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

export const isRightSidebarVisible = derived(
	uiStore,
	($ui) => $ui.rightSidebarOpen
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
