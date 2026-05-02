import { writable, derived } from 'svelte/store';
import type { Session, SessionDetail, Message } from '../api/client';

export type SessionMode = 'plan' | 'build';

export interface SessionState {
	sessions: Session[];
	currentSessionId: string | null;
	currentSession: SessionDetail | null;
	messages: Message[];
	isLoading: boolean;
	error: string | null;
	// Draft session state for new session flow
	isDraftSession: boolean;
	draftTitle: string;
	// Current session mode (plan/build)
	mode: SessionMode;
}

function createSessionStore() {
	const { subscribe, set, update } = writable<SessionState>({
		sessions: [],
		currentSessionId: null,
		currentSession: null,
		messages: [],
		isLoading: false,
		error: null,
		isDraftSession: false,
		draftTitle: '',
		mode: 'build'
	});

	return {
		subscribe,
		setSessions: (sessions: Session[]) =>
			update((s) => ({ ...s, sessions, error: null })),
		setCurrentSession: (session: SessionDetail | null) =>
			update((s) => ({ ...s, currentSession: session, currentSessionId: session?.session_id ?? null, isDraftSession: false })),
		setCurrentSessionId: (id: string | null) =>
			update((s) => ({ ...s, currentSessionId: id })),
		setMessages: (messages: Message[]) =>
			update((s) => ({ ...s, messages })),
		addMessage: (message: Message) =>
			update((s) => ({ ...s, messages: [...s.messages, message] })),
		updateMessageContent: (id: string, content: string) =>
			update((s) => ({
				...s,
				messages: s.messages.map((m) => (m.id === id ? { ...m, content: m.content + content } : m))
			})),
		setLoading: (isLoading: boolean) => update((s) => ({ ...s, isLoading })),
		setError: (error: string | null) => update((s) => ({ ...s, error })),
		clearError: () => update((s) => ({ ...s, error: null })),
		removeSession: (sessionId: string) =>
			update((s) => ({
				...s,
				sessions: s.sessions.filter((sess) => sess.session_id !== sessionId),
				currentSessionId: s.currentSessionId === sessionId ? null : s.currentSessionId,
				currentSession: s.currentSessionId === sessionId ? null : s.currentSession,
				isDraftSession: s.currentSessionId === sessionId ? false : s.isDraftSession
			})),
		// Mode methods
		setMode: (mode: SessionMode) =>
			update((s) => ({ ...s, mode })),
		toggleMode: () =>
			update((s) => ({
				...s,
				mode: s.mode === 'plan' ? 'build' : 'plan'
			})),
		// Draft session methods
		startDraftSession: (title: string = 'New Session') =>
			update((s) => ({
				...s,
				currentSessionId: null,
				currentSession: null,
				messages: [],
				isDraftSession: true,
				draftTitle: title,
				error: null
			})),
		commitDraftSession: (session: SessionDetail) =>
			update((s) => ({
				...s,
				currentSessionId: session.session_id,
				currentSession: session,
				isDraftSession: false,
				draftTitle: ''
			})),
		cancelDraftSession: () =>
			update((s) => ({
				...s,
				isDraftSession: false,
				draftTitle: ''
			})),
		reset: () =>
			set({
				sessions: [],
				currentSessionId: null,
				currentSession: null,
				messages: [],
				isLoading: false,
				error: null,
				isDraftSession: false,
				draftTitle: '',
				mode: 'build'
			})
	};
}

export const sessionStore = createSessionStore();

// Derived stores
export const currentSessionMessages = derived(sessionStore, ($store) => $store.messages);

export const isSessionActive = derived(
	sessionStore,
	($store) => $store.currentSessionId !== null || $store.isDraftSession
);

export const hasActiveOrDraftSession = derived(
	sessionStore,
	($store) => $store.currentSessionId !== null || $store.isDraftSession
);
