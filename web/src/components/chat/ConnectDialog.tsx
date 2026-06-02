import { useReducer, useEffect, useRef, useMemo, useCallback, useState } from "react";
import {
  X,
  Search,
  Plug,
  PlugZap,
  Plus,
  Trash2,
  AlertCircle,
  Check,
  Package,
  User,
} from "lucide-react";
import { api } from "../../api/client";
import type { ProviderInfo } from "../../types/api";

interface ConnectDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onConnect?: () => void;
}

type ViewMode = "list" | "apiKey" | "addProvider";

interface DialogState {
  providers: ProviderInfo[];
  loading: boolean;
  error: string | null;
  searchQuery: string;
  selectedIndex: number;
  viewMode: ViewMode;
  selectedProvider: ProviderInfo | null;
  apiKeyInput: string;
  isSubmitting: boolean;
}

type Action =
  | { type: "FETCH_START" }
  | { type: "FETCH_SUCCESS"; providers: ProviderInfo[] }
  | { type: "FETCH_ERROR"; error: string }
  | { type: "SET_SEARCH"; query: string }
  | { type: "SELECT_INDEX"; index: number }
  | { type: "NAV_UP"; max: number }
  | { type: "NAV_DOWN"; max: number }
  | { type: "SHOW_API_KEY"; provider: ProviderInfo }
  | { type: "SHOW_ADD_PROVIDER" }
  | { type: "SHOW_LIST" }
  | { type: "SET_API_KEY"; value: string }
  | { type: "SUBMIT_START" }
  | { type: "SUBMIT_END" }
  | { type: "UPDATE_PROVIDER"; provider: ProviderInfo };

function reducer(state: DialogState, action: Action): DialogState {
  switch (action.type) {
    case "FETCH_START":
      return { ...state, loading: true, error: null };
    case "FETCH_SUCCESS":
      return { ...state, providers: action.providers, loading: false };
    case "FETCH_ERROR":
      return { ...state, error: action.error, loading: false };
    case "SET_SEARCH":
      return { ...state, searchQuery: action.query, selectedIndex: 0 };
    case "SELECT_INDEX":
      return { ...state, selectedIndex: action.index };
    case "NAV_UP": {
      const prev = state.selectedIndex;
      return { ...state, selectedIndex: prev > 0 ? prev - 1 : action.max - 1 };
    }
    case "NAV_DOWN": {
      const prev = state.selectedIndex;
      return { ...state, selectedIndex: prev < action.max - 1 ? prev + 1 : 0 };
    }
    case "SHOW_API_KEY":
      return {
        ...state,
        viewMode: "apiKey",
        selectedProvider: action.provider,
        apiKeyInput: "",
      };
    case "SHOW_ADD_PROVIDER":
      return {
        ...state,
        viewMode: "addProvider",
        selectedProvider: null,
      };
    case "SHOW_LIST":
      return {
        ...state,
        viewMode: "list",
        selectedProvider: null,
        apiKeyInput: "",
      };
    case "SET_API_KEY":
      return { ...state, apiKeyInput: action.value };
    case "SUBMIT_START":
      return { ...state, isSubmitting: true };
    case "SUBMIT_END":
      return { ...state, isSubmitting: false };
    case "UPDATE_PROVIDER": {
      const updated = state.providers.map((p) =>
        p.id === action.provider.id ? action.provider : p,
      );
      return { ...state, providers: updated };
    }
  }
}

const initialState: DialogState = {
  providers: [],
  loading: true,
  error: null,
  searchQuery: "",
  selectedIndex: 0,
  viewMode: "list",
  selectedProvider: null,
  apiKeyInput: "",
  isSubmitting: false,
};

export function ConnectDialog({ isOpen, onClose, onConnect }: ConnectDialogProps) {
  const [state, dispatch] = useReducer(reducer, initialState);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const apiKeyInputRef = useRef<HTMLInputElement>(null);

  // Fetch providers when dialog opens
  useEffect(() => {
    if (!isOpen) return;

    dispatch({ type: "FETCH_START" });
    api
      .listProviders()
      .then((data) => dispatch({ type: "FETCH_SUCCESS", providers: data.providers }))
      .catch((err) =>
        dispatch({
          type: "FETCH_ERROR",
          error: err instanceof Error ? err.message : "Failed to load providers",
        }),
      );

    const raf = requestAnimationFrame(() => {
      searchInputRef.current?.focus();
    });
    return () => cancelAnimationFrame(raf);
  }, [isOpen]);

  // Focus API key input when showing API key view
  useEffect(() => {
    if (state.viewMode === "apiKey") {
      const raf = requestAnimationFrame(() => {
        apiKeyInputRef.current?.focus();
      });
      return () => cancelAnimationFrame(raf);
    }
  }, [state.viewMode]);

  // Filter providers based on search query
  const filteredProviders = useMemo(() => {
    const query = state.searchQuery.toLowerCase().trim();
    if (!query) return state.providers;

    return state.providers.filter(
      (p) => p.id.toLowerCase().includes(query) || p.display_name.toLowerCase().includes(query),
    );
  }, [state.providers, state.searchQuery]);

  // Add "Add New" option to filtered list
  const listItems = useMemo(() => {
    return [...filteredProviders, null]; // null represents "Add New" option
  }, [filteredProviders]);

  // Keyboard navigation
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (state.viewMode !== "list") return;

      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          dispatch({ type: "NAV_DOWN", max: listItems.length });
          break;
        case "ArrowUp":
          e.preventDefault();
          dispatch({ type: "NAV_UP", max: listItems.length });
          break;
        case "Enter": {
          e.preventDefault();
          const selected = listItems[state.selectedIndex];
          if (selected === null) {
            dispatch({ type: "SHOW_ADD_PROVIDER" });
          } else {
            dispatch({ type: "SHOW_API_KEY", provider: selected });
          }
          break;
        }
        case "Escape":
          e.preventDefault();
          onClose();
          break;
      }
    },
    [state.viewMode, state.selectedIndex, listItems, onClose],
  );

  // Handle connect/disconnect
  const handleToggleConnection = useCallback(
    async (provider: ProviderInfo) => {
      if (provider.connected) {
        // Disconnect
        try {
          await api.disconnectProvider(provider.id);
          dispatch({
            type: "UPDATE_PROVIDER",
            provider: { ...provider, connected: false },
          });
          onConnect?.();
        } catch (err) {
          dispatch({
            type: "FETCH_ERROR",
            error: err instanceof Error ? err.message : "Failed to disconnect",
          });
        }
      } else {
        // Show API key input
        dispatch({ type: "SHOW_API_KEY", provider });
      }
    },
    [onConnect],
  );

  // Handle API key submission
  const handleApiKeySubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!state.selectedProvider || !state.apiKeyInput.trim()) return;

      dispatch({ type: "SUBMIT_START" });
      try {
        await api.connectProvider(state.selectedProvider.id, {
          api_key: state.apiKeyInput.trim(),
        });
        dispatch({
          type: "UPDATE_PROVIDER",
          provider: { ...state.selectedProvider, connected: true },
        });
        dispatch({ type: "SHOW_LIST" });
        onConnect?.();
      } catch (err) {
        dispatch({
          type: "FETCH_ERROR",
          error: err instanceof Error ? err.message : "Failed to connect",
        });
      } finally {
        dispatch({ type: "SUBMIT_END" });
      }
    },
    [state.selectedProvider, state.apiKeyInput, onConnect],
  );

  // Handle delete provider
  const handleDeleteProvider = useCallback(
    async (provider: ProviderInfo) => {
      if (!confirm(`Delete provider "${provider.display_name}"?`)) return;

      try {
        await api.deleteProvider(provider.id);
        dispatch({
          type: "FETCH_SUCCESS",
          providers: state.providers.filter((p) => p.id !== provider.id),
        });
      } catch (err) {
        dispatch({
          type: "FETCH_ERROR",
          error: err instanceof Error ? err.message : "Failed to delete",
        });
      }
    },
    [state.providers],
  );

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div
        className="w-full max-w-2xl max-h-[80vh] flex flex-col rounded-xl bg-white shadow-2xl dark:bg-neutral-900"
        onKeyDown={handleKeyDown}
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-neutral-200 px-6 py-4 dark:border-neutral-800">
          <h2 className="text-lg font-semibold text-neutral-900 dark:text-neutral-100">
            {state.viewMode === "list" && "Connect Provider"}
            {state.viewMode === "apiKey" && `Connect to ${state.selectedProvider?.display_name}`}
            {state.viewMode === "addProvider" && "Add New Provider"}
          </h2>
          <button
            onClick={onClose}
            className="rounded p-1 text-neutral-500 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Close"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Error message */}
        {state.error && (
          <div className="mx-6 mt-4 flex items-center gap-2 rounded-lg bg-red-50 px-4 py-3 text-sm text-red-600 dark:bg-red-900/20 dark:text-red-400">
            <AlertCircle className="h-4 w-4" />
            {state.error}
          </div>
        )}

        {/* Content */}
        <div className="flex-1 overflow-hidden">
          {state.viewMode === "list" && (
            <>
              {/* Search */}
              <div className="border-b border-neutral-200 px-6 py-4 dark:border-neutral-800">
                <div className="relative">
                  <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-neutral-400" />
                  <input
                    ref={searchInputRef}
                    type="text"
                    value={state.searchQuery}
                    onChange={(e) => dispatch({ type: "SET_SEARCH", query: e.target.value })}
                    placeholder="Search providers..."
                    className="w-full rounded-lg border border-neutral-300 bg-white py-2 pl-10 pr-4 text-base text-neutral-900 placeholder-neutral-400 focus:border-neutral-500 focus:outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
                  />
                </div>
              </div>

              {/* Provider list */}
              <div className="overflow-y-auto px-2 py-2" style={{ maxHeight: "400px" }}>
                {state.loading ? (
                  <div className="flex items-center justify-center py-12">
                    <div className="h-8 w-8 animate-spin rounded-full border-2 border-neutral-300 border-t-neutral-600" />
                  </div>
                ) : listItems.length === 0 ? (
                  <div className="py-12 text-center text-neutral-500 dark:text-neutral-400">
                    No providers found
                  </div>
                ) : (
                  <div className="space-y-1">
                    {listItems.map((provider, index) => {
                      const isSelected = index === state.selectedIndex;
                      const isAddNew = provider === null;

                      if (isAddNew) {
                        return (
                          <button
                            key="__add_new__"
                            onClick={() => dispatch({ type: "SHOW_ADD_PROVIDER" })}
                            onMouseEnter={() => dispatch({ type: "SELECT_INDEX", index })}
                            className={`w-full flex items-center gap-3 rounded-lg px-4 py-3 text-left transition-colors ${
                              isSelected
                                ? "bg-neutral-100 dark:bg-neutral-800"
                                : "hover:bg-neutral-50 dark:hover:bg-neutral-800/50"
                            }`}
                          >
                            <div className="flex h-8 w-8 items-center justify-center rounded-full bg-blue-100 dark:bg-blue-900/30">
                              <Plus className="h-4 w-4 text-blue-600 dark:text-blue-400" />
                            </div>
                            <div className="flex-1">
                              <div className="font-medium text-neutral-900 dark:text-neutral-100">
                                Add New Provider
                              </div>
                              <div className="text-xs text-neutral-500 dark:text-neutral-400">
                                Create a custom OpenAI-compatible provider
                              </div>
                            </div>
                          </button>
                        );
                      }

                      return (
                        <div
                          key={provider.id}
                          onMouseEnter={() => dispatch({ type: "SELECT_INDEX", index })}
                          className={`flex items-center gap-3 rounded-lg px-4 py-3 transition-colors ${
                            isSelected
                              ? "bg-neutral-100 dark:bg-neutral-800"
                              : "hover:bg-neutral-50 dark:hover:bg-neutral-800/50"
                          }`}
                        >
                          <div className="flex h-8 w-8 items-center justify-center rounded-full bg-neutral-100 dark:bg-neutral-800">
                            {provider.source === "bundled" ? (
                              <Package className="h-4 w-4 text-neutral-500" />
                            ) : (
                              <User className="h-4 w-4 text-neutral-500" />
                            )}
                          </div>
                          <div className="flex-1 min-w-0">
                            <div className="flex items-center gap-2">
                              <span className="font-medium text-neutral-900 dark:text-neutral-100 truncate">
                                {provider.display_name}
                              </span>
                              {provider.connected && <Check className="h-4 w-4 text-green-500" />}
                            </div>
                            <div className="text-xs text-neutral-500 dark:text-neutral-400 truncate">
                              {provider.id} · {provider.models.length} models
                            </div>
                          </div>
                          <div className="flex items-center gap-1">
                            {provider.source === "user" && (
                              <button
                                onClick={() => handleDeleteProvider(provider)}
                                className="rounded p-2 text-neutral-400 hover:bg-red-100 hover:text-red-600 dark:hover:bg-red-900/30 dark:hover:text-red-400"
                                title="Delete provider"
                              >
                                <Trash2 className="h-4 w-4" />
                              </button>
                            )}
                            <button
                              onClick={() => handleToggleConnection(provider)}
                              className={`rounded p-2 ${
                                provider.connected
                                  ? "text-green-600 hover:bg-green-100 dark:text-green-400 dark:hover:bg-green-900/30"
                                  : "text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-700"
                              }`}
                              title={provider.connected ? "Disconnect" : "Connect"}
                            >
                              {provider.connected ? (
                                <PlugZap className="h-4 w-4" />
                              ) : (
                                <Plug className="h-4 w-4" />
                              )}
                            </button>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            </>
          )}

          {state.viewMode === "apiKey" && state.selectedProvider && (
            <div className="px-6 py-6">
              <form onSubmit={handleApiKeySubmit} className="space-y-4">
                <div>
                  <label className="block text-sm font-medium text-neutral-700 dark:text-neutral-300 mb-2">
                    API Key for {state.selectedProvider.display_name}
                  </label>
                  <input
                    ref={apiKeyInputRef}
                    type="password"
                    value={state.apiKeyInput}
                    onChange={(e) => dispatch({ type: "SET_API_KEY", value: e.target.value })}
                    placeholder="Enter your API key"
                    className="w-full rounded-lg border border-neutral-300 bg-white px-4 py-2 text-base text-neutral-900 placeholder-neutral-400 focus:border-neutral-500 focus:outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
                  />
                  <p className="mt-2 text-xs text-neutral-500 dark:text-neutral-400">
                    Your API key will be stored securely and used for API requests.
                  </p>
                </div>
                <div className="flex gap-3">
                  <button
                    type="button"
                    onClick={() => dispatch({ type: "SHOW_LIST" })}
                    className="rounded-lg border border-neutral-300 px-4 py-2 text-sm font-medium text-neutral-700 hover:bg-neutral-50 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800"
                  >
                    Cancel
                  </button>
                  <button
                    type="submit"
                    disabled={!state.apiKeyInput.trim() || state.isSubmitting}
                    className="flex-1 rounded-lg bg-neutral-900 px-4 py-2 text-sm font-medium text-white hover:bg-neutral-800 disabled:opacity-50 disabled:cursor-not-allowed dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
                  >
                    {state.isSubmitting ? "Connecting..." : "Connect"}
                  </button>
                </div>
              </form>
            </div>
          )}

          {state.viewMode === "addProvider" && (
            <AddProviderForm
              onCancel={() => dispatch({ type: "SHOW_LIST" })}
              onSuccess={() => {
                dispatch({ type: "FETCH_START" });
                api
                  .listProviders()
                  .then((data) =>
                    dispatch({
                      type: "FETCH_SUCCESS",
                      providers: data.providers,
                    }),
                  )
                  .then(() => dispatch({ type: "SHOW_LIST" }))
                  .then(() => onConnect?.());
              }}
              onError={(error) => dispatch({ type: "FETCH_ERROR", error })}
            />
          )}
        </div>

        {/* Footer */}
        {state.viewMode === "list" && (
          <div className="hidden items-center justify-between border-t border-neutral-200 px-6 py-3 text-xs text-neutral-500 md:flex dark:border-neutral-800 dark:text-neutral-400">
            <div className="flex items-center gap-4">
              <span>
                <kbd className="rounded border border-neutral-300 px-1 font-mono dark:border-neutral-600">
                  ↑↓
                </kbd>{" "}
                Navigate
              </span>
              <span>
                <kbd className="rounded border border-neutral-300 px-1 font-mono dark:border-neutral-600">
                  Enter
                </kbd>{" "}
                Select
              </span>
              <span>
                <kbd className="rounded border border-neutral-300 px-1 font-mono dark:border-neutral-600">
                  Esc
                </kbd>{" "}
                Close
              </span>
            </div>
            <span>{filteredProviders.length} providers</span>
          </div>
        )}
      </div>
    </div>
  );
}

// Add Provider Form Component
interface AddProviderFormProps {
  onCancel: () => void;
  onSuccess: () => void;
  onError: (error: string) => void;
}

function AddProviderForm({ onCancel, onSuccess, onError }: AddProviderFormProps) {
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [formData, setFormData] = useState({
    provider_id: "",
    display_name: "",
    base_url: "",
    api_key: "",
    model_id: "",
    model_display_name: "",
    context_window: "128000",
    max_output_tokens: "32768",
    temperature: "1.0",
    supports_images: false,
  });

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setIsSubmitting(true);

    try {
      await api.createProvider({
        provider_id: formData.provider_id,
        display_name: formData.display_name,
        base_url: formData.base_url,
        api_key: formData.api_key,
        models: [
          {
            model_id: formData.model_id,
            display_name: formData.model_display_name || formData.model_id,
            context_window: parseInt(formData.context_window, 10) || 128000,
            max_output_tokens: parseInt(formData.max_output_tokens, 10) || 32768,
            temperature: parseFloat(formData.temperature) || 1.0,
            supports_images: formData.supports_images,
          },
        ],
      });
      onSuccess();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Failed to create provider");
    } finally {
      setIsSubmitting(false);
    }
  };

  const inputClass =
    "w-full rounded-lg border border-neutral-300 bg-white px-3 py-2 text-base text-neutral-900 placeholder-neutral-400 focus:border-neutral-500 focus:outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100";

  const labelClass = "block text-sm font-medium text-neutral-700 dark:text-neutral-300 mb-1";

  return (
    <form
      onSubmit={handleSubmit}
      className="px-6 py-4 overflow-y-auto"
      style={{ maxHeight: "400px" }}
    >
      <div className="space-y-4">
        {/* Provider Info Section */}
        <div className="space-y-3">
          <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
            Provider Information
          </h3>
          <div>
            <label className={labelClass}>Provider ID *</label>
            <input
              type="text"
              value={formData.provider_id}
              onChange={(e) => setFormData({ ...formData, provider_id: e.target.value })}
              placeholder="e.g., my-custom-provider"
              className={inputClass}
              required
            />
            <p className="mt-1 text-xs text-neutral-500">
              Unique identifier (lowercase letters, numbers, -, _)
            </p>
          </div>
          <div>
            <label className={labelClass}>Display Name *</label>
            <input
              type="text"
              value={formData.display_name}
              onChange={(e) => setFormData({ ...formData, display_name: e.target.value })}
              placeholder="e.g., My Custom Provider"
              className={inputClass}
              required
            />
          </div>
          <div>
            <label className={labelClass}>Base URL *</label>
            <input
              type="url"
              value={formData.base_url}
              onChange={(e) => setFormData({ ...formData, base_url: e.target.value })}
              placeholder="https://api.example.com/v1"
              className={inputClass}
              required
            />
          </div>
          <div>
            <label className={labelClass}>API Key *</label>
            <input
              type="password"
              value={formData.api_key}
              onChange={(e) => setFormData({ ...formData, api_key: e.target.value })}
              placeholder="Your API key"
              className={inputClass}
              required
            />
          </div>
        </div>

        {/* Model Info Section */}
        <div className="space-y-3 pt-4 border-t border-neutral-200 dark:border-neutral-800">
          <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
            Model Configuration
          </h3>
          <div className="grid grid-cols-2 gap-3">
            <div className="col-span-2">
              <label className={labelClass}>Model ID *</label>
              <input
                type="text"
                value={formData.model_id}
                onChange={(e) => setFormData({ ...formData, model_id: e.target.value })}
                placeholder="e.g., gpt-4"
                className={inputClass}
                required
              />
            </div>
            <div className="col-span-2">
              <label className={labelClass}>Model Display Name</label>
              <input
                type="text"
                value={formData.model_display_name}
                onChange={(e) =>
                  setFormData({
                    ...formData,
                    model_display_name: e.target.value,
                  })
                }
                placeholder="e.g., GPT-4"
                className={inputClass}
              />
            </div>
            <div>
              <label className={labelClass}>Context Window</label>
              <input
                type="number"
                value={formData.context_window}
                onChange={(e) => setFormData({ ...formData, context_window: e.target.value })}
                placeholder="128000"
                className={inputClass}
              />
            </div>
            <div>
              <label className={labelClass}>Max Output Tokens</label>
              <input
                type="number"
                value={formData.max_output_tokens}
                onChange={(e) =>
                  setFormData({
                    ...formData,
                    max_output_tokens: e.target.value,
                  })
                }
                placeholder="32768"
                className={inputClass}
              />
            </div>
            <div>
              <label className={labelClass}>Temperature</label>
              <input
                type="number"
                step="0.1"
                min="0"
                max="2"
                value={formData.temperature}
                onChange={(e) => setFormData({ ...formData, temperature: e.target.value })}
                placeholder="1.0"
                className={inputClass}
              />
            </div>
            <div className="flex items-center">
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={formData.supports_images}
                  onChange={(e) =>
                    setFormData({
                      ...formData,
                      supports_images: e.target.checked,
                    })
                  }
                  className="rounded border-neutral-300 dark:border-neutral-700"
                />
                <span className="text-sm text-neutral-700 dark:text-neutral-300">
                  Supports images
                </span>
              </label>
            </div>
          </div>
        </div>
      </div>

      {/* Actions */}
      <div className="flex gap-3 pt-6 mt-4 border-t border-neutral-200 dark:border-neutral-800">
        <button
          type="button"
          onClick={onCancel}
          className="rounded-lg border border-neutral-300 px-4 py-2 text-sm font-medium text-neutral-700 hover:bg-neutral-50 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800"
        >
          Cancel
        </button>
        <button
          type="submit"
          disabled={isSubmitting}
          className="flex-1 rounded-lg bg-neutral-900 px-4 py-2 text-sm font-medium text-white hover:bg-neutral-800 disabled:opacity-50 disabled:cursor-not-allowed dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
        >
          {isSubmitting ? "Creating..." : "Create Provider"}
        </button>
      </div>
    </form>
  );
}
