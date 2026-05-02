import { writable, derived } from 'svelte/store';
import type { Session, SessionDetail, Message } from '../api/client';

export interface SessionState {
	sessions: Session[];
	currentSessionId: string | null;
	currentSession: SessionDetail | null;
	messages: Message[];
	isLoading: boolean;
	error: string | null;
}

function createSessionStore() {
	const { subscribe, set, update } = writable<SessionState>({
		sessions: [],
		currentSessionId: null,
		currentSession: null,
		messages: [],
		isLoading: false,
		error: null
	});

	return {
		subscribe,
		setSessions: (sessions: Session[]) =>
			update((s) => ({ ...s, sessions, error: null })),
		setCurrentSession: (session: SessionDetail | null) =>
			update((s) => ({ ...s, currentSession: session, currentSessionId: session?.session_id ?? null })),
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
				currentSession: s.currentSessionId === sessionId ? null : s.currentSession
			})),
		reset: () =>
			set({
				sessions: [],
				currentSessionId: null,
				currentSession: null,
				messages: [],
				isLoading: false,
				error: null
			})
	};
}

export const sessionStore = createSessionStore();

// Derived stores
export const currentSessionMessages = derived(sessionStore, ($store) => $store.messages);

export const isSessionActive = derived(
	sessionStore,
	($store) => $store.currentSessionId !== null
);
