import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import {
  X,
  Search,
  Check,
  Bot,
  Book,
  Eye,
  Palette,
  Wrench,
  Database,
  Sparkles,
} from "lucide-react";
import { api } from "../../api/client";
import type { ModelInfo } from "../../types/api";

interface ModelPanelProps {
  isOpen: boolean;
  onClose: () => void;
  /** Currently selected global model ID */
  currentModelId: string | null;
  /** Currently selected global provider ID */
  currentProviderId: string | null;
  /** Called when the global (General tab) model is changed */
  onModelChange?: (model: ModelInfo) => void;
}

interface TabInfo {
  id: string;
  label: string;
  icon: React.ReactNode;
}

const AGENT_TAB_ITEMS: TabInfo[] = [
  { id: "general", label: "General", icon: <Bot className="h-4 w-4" /> },
  { id: "explorer", label: "Explorer", icon: <Search className="h-4 w-4" /> },
  { id: "librarian", label: "Librarian", icon: <Book className="h-4 w-4" /> },
  { id: "oracle", label: "Oracle", icon: <Sparkles className="h-4 w-4" /> },
  { id: "designer", label: "Designer", icon: <Palette className="h-4 w-4" /> },
  { id: "fixer", label: "Fixer", icon: <Wrench className="h-4 w-4" /> },
];

const MEMORY_TAB_ITEM: TabInfo = {
  id: "memory",
  label: "Memory",
  icon: <Database className="h-4 w-4" />,
};

export function ModelPanel({
  isOpen,
  onClose,
  currentModelId,
  currentProviderId,
  onModelChange,
}: ModelPanelProps) {
  const [activeTab, setActiveTab] = useState<string>("general");
  const [searchQuery, setSearchQuery] = useState("");
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [agentModels, setAgentModels] = useState<Record<string, string>>({});
  const [memoryModelStr, setMemoryModelStr] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const navRef = useRef<HTMLDivElement>(null);
  const [activeRect, setActiveRect] = useState<{
    top: number;
    height: number;
  } | null>(null);

  // Fetch models, agent config, and memory config when panel opens
  useEffect(() => {
    if (!isOpen) return;

    Promise.all([api.listModels(), api.getAgentModels(), api.getMemoryModel()])
      .then(([modelsResp, agentModelsResp, memoryResp]) => {
        setIsLoading(true);
        setModels(modelsResp.models ?? []);
        setAgentModels(agentModelsResp.agent_models);
        setMemoryModelStr(memoryResp.model_str);
      })
      .catch((err) => {
        console.error("Failed to load model data:", err);
      })
      .finally(() => {
        setIsLoading(false);
      });

    // Reset state on open — defer to avoid cascading renders
    const rafId = requestAnimationFrame(() => {
      setSearchQuery("");
      setActiveTab("general");
    });
    return () => cancelAnimationFrame(rafId);
  }, [isOpen]);

  // Focus search input when panel opens or tab changes
  useEffect(() => {
    if (isOpen && searchInputRef.current) {
      setTimeout(() => searchInputRef.current?.focus(), 100);
    }
  }, [isOpen, activeTab]);

  // Measure active button position for sliding highlight indicator
  useEffect(() => {
    if (navRef.current) {
      const activeEl = navRef.current.querySelector<HTMLElement>(
        `[data-tab-id="${activeTab}"]`,
      );
      if (activeEl) {
        const navRect = navRef.current.getBoundingClientRect();
        const btnRect = activeEl.getBoundingClientRect();
        setActiveRect({
          top: btnRect.top - navRect.top,
          height: btnRect.height,
        });
      }
    }
  }, [activeTab]);

  // Close on Escape
  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  // Compute the model label shown beneath each tab in the sidebar
  const getTabLabel = useCallback(
    (tabId: string): string => {
      if (tabId === "general") {
        if (currentModelId && currentProviderId) {
          const m = models.find(
            (m) =>
              m.id === currentModelId && m.provider_id === currentProviderId,
          );
          return m?.display_name || `${currentProviderId}/${currentModelId}`;
        }
        return "No model";
      }
      if (tabId === "memory") {
        if (memoryModelStr) {
          return memoryModelStr.split("/").pop() || memoryModelStr;
        }
        return "<inherit>";
      }
      // Agent tab
      const overrideStr = agentModels[tabId];
      if (overrideStr) {
        return overrideStr.split("/").pop() || overrideStr;
      }
      return "<inherit>";
    },
    [currentModelId, currentProviderId, models, agentModels, memoryModelStr],
  );

  // Compute the currently "active" model for the selected tab
  const activeModelForTab = useMemo((): {
    provider_id: string;
    model_id: string;
    label: string;
  } | null => {
    if (activeTab === "general") {
      if (currentModelId && currentProviderId) {
        const m = models.find(
          (m) => m.id === currentModelId && m.provider_id === currentProviderId,
        );
        return {
          provider_id: currentProviderId,
          model_id: currentModelId,
          label: m?.display_name || `${currentProviderId}/${currentModelId}`,
        };
      }
      return null;
    }

    if (activeTab === "memory") {
      if (!memoryModelStr) return null;
      const parts = memoryModelStr.split("/");
      if (parts.length !== 2) return null;
      const [providerId, modelId] = parts;
      const m = models.find(
        (m) => m.id === modelId && m.provider_id === providerId,
      );
      return {
        provider_id: providerId,
        model_id: modelId,
        label: m?.display_name || memoryModelStr,
      };
    }

    // Agent tab
    const overrideStr = agentModels[activeTab];
    if (!overrideStr) return null;
    const parts = overrideStr.split("/");
    if (parts.length !== 2) return null;
    const [providerId, modelId] = parts;
    const m = models.find(
      (m) => m.id === modelId && m.provider_id === providerId,
    );
    return {
      provider_id: providerId,
      model_id: modelId,
      label: m?.display_name || overrideStr,
    };
  }, [
    activeTab,
    currentModelId,
    currentProviderId,
    models,
    agentModels,
    memoryModelStr,
  ]);

  // Filter models by search query
  const filteredModels = useMemo(() => {
    if (!searchQuery.trim()) return models;
    const q = searchQuery.toLowerCase();
    return models.filter(
      (m) =>
        m.display_name.toLowerCase().includes(q) ||
        m.id.toLowerCase().includes(q) ||
        m.provider_name.toLowerCase().includes(q) ||
        m.provider_id.toLowerCase().includes(q),
    );
  }, [models, searchQuery]);

  // Group filtered models by provider
  const groupedModels = useMemo(() => {
    const map = new Map<string, ModelInfo[]>();
    for (const model of filteredModels) {
      const key = model.provider_name || model.provider_id;
      const list = map.get(key);
      if (list) {
        list.push(model);
      } else {
        map.set(key, [model]);
      }
    }
    return map;
  }, [filteredModels]);

  // Handle model selection based on active tab
  const handleSelectModel = useCallback(
    async (model: ModelInfo) => {
      if (activeTab === "general") {
        try {
          await api.setDefaultModel({
            provider_id: model.provider_id,
            model_id: model.id,
          });
          onModelChange?.(model);
          onClose();
        } catch (err) {
          console.error("Failed to set default model:", err);
        }
      } else if (activeTab === "memory") {
        const modelStr = `${model.provider_id}/${model.id}`;
        try {
          await api.setMemoryModel({
            role: "consolidation",
            model_str: modelStr,
          });
          setMemoryModelStr(modelStr);
        } catch (err) {
          console.error("Failed to set memory model:", err);
        }
      } else {
        const modelStr = `${model.provider_id}/${model.id}`;
        try {
          await api.setAgentModel({
            agent_type: activeTab,
            model_str: modelStr,
          });
          setAgentModels((prev) => ({ ...prev, [activeTab]: modelStr }));
        } catch (err) {
          console.error(`Failed to set agent model for ${activeTab}:`, err);
        }
      }
    },
    [activeTab, onModelChange, onClose],
  );

  // Clear override for agent or memory tab
  const handleClearOverride = useCallback(async () => {
    if (activeTab === "general") return;

    if (activeTab === "memory") {
      try {
        await api.setMemoryModel({
          role: "consolidation",
          model_str: "",
        });
        setMemoryModelStr(null);
      } catch (err) {
        console.error("Failed to clear memory model:", err);
      }
    } else {
      try {
        await api.setAgentModel({
          agent_type: activeTab,
          model_str: "",
        });
        setAgentModels((prev) => {
          const next = { ...prev };
          delete next[activeTab];
          return next;
        });
      } catch (err) {
        console.error(`Failed to clear agent model for ${activeTab}:`, err);
      }
    }
  }, [activeTab]);

  if (!isOpen) return null;

  const activeModel = activeModelForTab;

  const isSelectedModel = (model: ModelInfo): boolean => {
    if (activeTab === "general") {
      return (
        model.id === currentModelId && model.provider_id === currentProviderId
      );
    }
    if (activeTab === "memory") {
      return memoryModelStr === `${model.provider_id}/${model.id}`;
    }
    return agentModels[activeTab] === `${model.provider_id}/${model.id}`;
  };

  const canClearOverride =
    activeTab !== "general" &&
    (activeTab === "memory" ? !!memoryModelStr : !!agentModels[activeTab]);

  return (
    <div
      className="fixed inset-0 z-[9998] flex items-start justify-center bg-black/30 pt-[10vh] sm:pt-[8vh]"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={panelRef}
        className="mx-2 flex w-full max-w-xl flex-col rounded-xl border border-neutral-200 bg-white shadow-2xl dark:border-neutral-700 dark:bg-neutral-900 sm:mx-4"
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-700">
          <h2 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
            Model Configuration
          </h2>
          <button
            onClick={onClose}
            className="rounded p-1 text-neutral-500 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Close"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Body: sidebar + content */}
        <div className="flex min-h-0 flex-1">
          {/* Sidebar */}
          <nav
            ref={navRef}
            className="relative w-40 shrink-0 overflow-y-auto border-r border-neutral-200 p-2 dark:border-neutral-800"
          >
            {/* Sliding highlight indicator */}
            {activeRect && (
              <div
                className="absolute left-2 right-2 rounded-md bg-neutral-100 transition-all duration-200 dark:bg-neutral-800"
                style={{ top: activeRect.top, height: activeRect.height }}
              />
            )}
            {AGENT_TAB_ITEMS.map((tab) => {
              const isActive = activeTab === tab.id;
              const label = getTabLabel(tab.id);
              return (
                <button
                  key={tab.id}
                  data-tab-id={tab.id}
                  onClick={() => setActiveTab(tab.id)}
                  className={`relative flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors duration-150 ${
                    isActive
                      ? "font-medium text-neutral-900 dark:text-neutral-100"
                      : "text-neutral-500 hover:bg-neutral-50 hover:text-neutral-700 dark:text-neutral-400 dark:hover:bg-neutral-800/50 dark:hover:text-neutral-300"
                  }`}
                >
                  {tab.icon}
                  <div className="flex flex-col items-start overflow-hidden">
                    <span>{tab.label}</span>
                    <span className="max-w-[90px] truncate text-[10px] text-neutral-400 dark:text-neutral-500">
                      {label}
                    </span>
                  </div>
                </button>
              );
            })}

            {/* Divider between agent tabs and module settings */}
            <div className="my-1 border-t border-neutral-200 dark:border-neutral-700" />

            {/* Memory module setting */}
            {(() => {
              const isActive = activeTab === MEMORY_TAB_ITEM.id;
              const label = getTabLabel(MEMORY_TAB_ITEM.id);
              return (
                <button
                  key={MEMORY_TAB_ITEM.id}
                  data-tab-id={MEMORY_TAB_ITEM.id}
                  onClick={() => setActiveTab(MEMORY_TAB_ITEM.id)}
                  className={`relative flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors duration-150 ${
                    isActive
                      ? "font-medium text-neutral-900 dark:text-neutral-100"
                      : "text-neutral-500 hover:bg-neutral-50 hover:text-neutral-700 dark:text-neutral-400 dark:hover:bg-neutral-800/50 dark:hover:text-neutral-300"
                  }`}
                >
                  {MEMORY_TAB_ITEM.icon}
                  <div className="flex flex-col items-start overflow-hidden">
                    <span>{MEMORY_TAB_ITEM.label}</span>
                    <span className="max-w-[90px] truncate text-[10px] text-neutral-400 dark:text-neutral-500">
                      {label}
                    </span>
                  </div>
                </button>
              );
            })()}
          </nav>

          {/* Content */}
          <div className="flex min-w-0 flex-1 flex-col">
            {/* Search */}
            <div className="p-3">
              <div className="relative">
                <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-neutral-400" />
                <input
                  ref={searchInputRef}
                  type="text"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  placeholder="Search models by provider or model name..."
                  className="w-full rounded-lg border border-neutral-300 bg-neutral-50 py-2 pl-8 pr-3 text-base text-neutral-900 placeholder-neutral-400 outline-none focus:border-neutral-500 focus:ring-1 focus:ring-neutral-500 dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-100 dark:placeholder-neutral-500 dark:focus:border-neutral-400"
                />
              </div>
            </div>

            {/* Memory role pills (only when Memory tab is active) */}
            {activeTab === "memory" && (
              <div className="flex items-center gap-2 border-b border-neutral-100 px-3 py-1.5 dark:border-neutral-800">
                <div className="inline-flex items-center gap-1.5 rounded-md bg-neutral-100 px-2.5 py-1 dark:bg-neutral-800">
                  <Database className="h-3.5 w-3.5 text-neutral-500" />
                  <span className="text-xs font-medium text-neutral-700 dark:text-neutral-300">
                    Consolidation
                  </span>
                  <span className="text-[10px] text-neutral-400">
                    {memoryModelStr
                      ? memoryModelStr.split("/").pop()
                      : "<inherit>"}
                  </span>
                </div>
              </div>
            )}

            {/* Model list */}
            <div className="max-h-[45vh] min-h-[200px] overflow-y-auto border-t border-neutral-100 px-1 dark:border-neutral-800">
              {isLoading ? (
                <div className="flex items-center justify-center py-8">
                  <div className="h-5 w-5 animate-spin rounded-full border-2 border-neutral-300 border-t-neutral-600 dark:border-neutral-600 dark:border-t-neutral-300" />
                </div>
              ) : groupedModels.size === 0 ? (
                <div className="py-8 text-center text-xs text-neutral-500 dark:text-neutral-400">
                  {searchQuery
                    ? "No models match your search"
                    : "No models available"}
                </div>
              ) : (
                Array.from(groupedModels.entries()).map(
                  ([provider, providerModels]) => (
                    <div key={provider}>
                      <div className="sticky top-0 bg-white px-3 py-1.5 text-[11px] font-medium uppercase tracking-wider text-neutral-500 dark:bg-neutral-900 dark:text-neutral-400">
                        {provider}
                      </div>
                      {providerModels.map((model) => {
                        const isSelected = isSelectedModel(model);

                        return (
                          <div key={`${model.provider_id}/${model.id}`}>
                            <button
                              onClick={() => handleSelectModel(model)}
                              className={`flex w-full items-center gap-3 rounded-md px-3 py-2 text-left text-xs transition-colors hover:bg-neutral-100 dark:hover:bg-neutral-800 ${
                                isSelected
                                  ? "bg-blue-50 dark:bg-blue-900/20"
                                  : ""
                              }`}
                            >
                              <span
                                className={`flex-1 font-medium ${
                                  isSelected
                                    ? "text-blue-700 dark:text-blue-300"
                                    : "text-neutral-900 dark:text-neutral-100"
                                }`}
                              >
                                {model.display_name}
                              </span>
                              {model.supports_vision && (
                                <Eye className="h-3.5 w-3.5 text-neutral-400" />
                              )}
                              {isSelected && (
                                <Check className="h-3.5 w-3.5 text-blue-600 dark:text-blue-400" />
                              )}
                            </button>
                          </div>
                        );
                      })}{" "}
                    </div>
                  ),
                )
              )}
            </div>
          </div>
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between border-t border-neutral-200 px-4 py-2.5 dark:border-neutral-700">
          <div className="flex items-center gap-2 text-[11px] text-neutral-500 dark:text-neutral-400">
            {activeModel ? (
              <>
                <span className="font-medium text-neutral-700 dark:text-neutral-300">
                  Current:
                </span>
                <span className="truncate max-w-[200px]">
                  {activeModel.label}
                </span>
              </>
            ) : (
              <span className="italic">
                {activeTab === "general"
                  ? "No model selected"
                  : activeTab === "memory"
                    ? "Using default model"
                    : "Using parent session model (<inherit>)"}
              </span>
            )}
          </div>
          <div className="flex items-center gap-2">
            {canClearOverride && (
              <button
                onClick={handleClearOverride}
                className="rounded px-2 py-1 text-[11px] text-neutral-600 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
              >
                Clear override
              </button>
            )}
            <button
              onClick={onClose}
              className="rounded bg-neutral-900 px-3 py-1 text-xs text-white hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
            >
              Done
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
