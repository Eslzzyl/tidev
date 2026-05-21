import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import { X, Search, Camera, Check } from "lucide-react";
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
}

const AGENT_TABS: TabInfo[] = [
  { id: "general", label: "General" },
  { id: "explorer", label: "Explorer" },
  { id: "librarian", label: "Librarian" },
  { id: "oracle", label: "Oracle" },
  { id: "designer", label: "Designer" },
  { id: "fixer", label: "Fixer" },
];

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
  const [agentThinkingLevels, setAgentThinkingLevels] = useState<
    Record<string, string>
  >({});
  const [isLoading, setIsLoading] = useState(false);
  const [expandedModelId, setExpandedModelId] = useState<string | null>(null);
  const [selectedThinking, setSelectedThinking] = useState<string>("");
  const searchInputRef = useRef<HTMLInputElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  // Fetch models and agent config when panel opens
  useEffect(() => {
    if (!isOpen) return;

    setIsLoading(true);
    Promise.all([api.listModels(), api.getAgentModels()])
      .then(([modelsResp, agentModelsResp]) => {
        setModels(modelsResp.models ?? []);
        setAgentModels(agentModelsResp.agent_models);
        // Store thinking levels for agents if present
        if (agentModelsResp.agent_thinking_levels) {
          setAgentThinkingLevels(agentModelsResp.agent_thinking_levels);
        }
      })
      .catch((err) => {
        console.error("Failed to load model data:", err);
      })
      .finally(() => {
        setIsLoading(false);
      });

    // Reset search and tab
    setSearchQuery("");
    setActiveTab("general");
    setExpandedModelId(null);
  }, [isOpen]);

  // Focus search input when panel opens
  useEffect(() => {
    if (isOpen && searchInputRef.current) {
      setTimeout(() => searchInputRef.current?.focus(), 100);
    }
  }, [isOpen, activeTab]);

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

    // Agent tab
    const overrideStr = agentModels[activeTab];
    if (!overrideStr) return null; // <inherit>

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
  }, [activeTab, currentModelId, currentProviderId, models, agentModels]);

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

  // Handle model selection
  const handleSelectModel = useCallback(
    async (model: ModelInfo) => {
      // If model supports thinking and not already expanded for this model, expand
      if (
        model.thinking_supported &&
        expandedModelId !== `${model.provider_id}/${model.id}`
      ) {
        setExpandedModelId(`${model.provider_id}/${model.id}`);
        setSelectedThinking(
          model.thinking_options.includes(model.thinking_level)
            ? model.thinking_level
            : model.thinking_options[0],
        );
        return;
      }

      // Confirm selection with (or without) thinking level
      const tl = model.thinking_supported ? selectedThinking : "";

      if (activeTab === "general") {
        // Set default model immediately
        try {
          await api.setDefaultModel({
            provider_id: model.provider_id,
            model_id: model.id,
            thinking_level: tl || undefined,
          });
          onModelChange?.(model);
          onClose();
        } catch (err) {
          console.error("Failed to set default model:", err);
        }
      } else {
        // Set agent model override immediately
        const modelStr = `${model.provider_id}/${model.id}`;
        try {
          await api.setAgentModel({
            agent_type: activeTab,
            model_str: modelStr,
            thinking_level: tl || undefined,
          });
          // Update local state so the tab shows the new label
          setAgentModels((prev) => ({ ...prev, [activeTab]: modelStr }));
          if (tl) {
            setAgentThinkingLevels((prev) => ({ ...prev, [activeTab]: tl }));
          }
        } catch (err) {
          console.error(`Failed to set agent model for ${activeTab}:`, err);
        }
      }
      setExpandedModelId(null);
    },
    [activeTab, onModelChange, onClose, expandedModelId, selectedThinking],
  );

  // Clear agent model override
  const handleClearOverride = useCallback(async () => {
    if (activeTab === "general") return;
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
  }, [activeTab]);

  if (!isOpen) return null;

  const activeModel = activeModelForTab;

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

        {/* Tabs */}
        <div className="flex gap-0 border-b border-neutral-200 overflow-x-auto px-2 dark:border-neutral-700">
          {AGENT_TABS.map((tab) => {
            const isActive = activeTab === tab.id;
            const label =
              tab.id === "general"
                ? activeModel?.label || "Select model"
                : agentModels[tab.id]
                  ? agentModels[tab.id].split("/").pop() || agentModels[tab.id]
                  : "<inherit>";

            return (
              <button
                key={tab.id}
                onClick={() => setActiveTab(tab.id)}
                className={`flex flex-col items-center gap-0.5 px-3 py-2 text-xs font-medium transition-colors whitespace-nowrap ${
                  isActive
                    ? "border-b-2 border-neutral-900 text-neutral-900 dark:border-neutral-100 dark:text-neutral-100"
                    : "text-neutral-500 hover:text-neutral-700 dark:text-neutral-400 dark:hover:text-neutral-300"
                }`}
              >
                <span>{tab.label}</span>
                <span className="max-w-[100px] truncate text-[10px] text-neutral-400 dark:text-neutral-500">
                  {label}
                </span>
              </button>
            );
          })}
        </div>

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
              className="w-full rounded-lg border border-neutral-300 bg-neutral-50 py-2 pl-8 pr-3 text-xs text-neutral-900 placeholder-neutral-400 outline-none focus:border-neutral-500 focus:ring-1 focus:ring-neutral-500 dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-100 dark:placeholder-neutral-500 dark:focus:border-neutral-400"
            />
          </div>
        </div>

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
                    const isSelected =
                      activeTab === "general"
                        ? model.id === currentModelId &&
                          model.provider_id === currentProviderId
                        : agentModels[activeTab] ===
                          `${model.provider_id}/${model.id}`;
                    const isExpanded =
                      expandedModelId === `${model.provider_id}/${model.id}`;

                    return (
                      <div key={`${model.provider_id}/${model.id}`}>
                        <button
                          onClick={() => handleSelectModel(model)}
                          className={`flex w-full items-center gap-3 rounded-md px-3 py-2 text-left text-xs transition-colors hover:bg-neutral-100 dark:hover:bg-neutral-800 ${
                            isSelected ? "bg-blue-50 dark:bg-blue-900/20" : ""
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
                            <Camera className="h-3.5 w-3.5 text-neutral-400" />
                          )}
                          {isSelected && (
                            <Check className="h-3.5 w-3.5 text-blue-600 dark:text-blue-400" />
                          )}
                        </button>
                        {/* Thinking level sub-menu when expanded */}
                        {isExpanded && model.thinking_supported && (
                          <div className="ml-6 border-l-2 border-amber-200 pl-3 py-1 dark:border-amber-700">
                            {model.thinking_options.map((opt) => {
                              const parts = opt.split(":");
                              const label = parts[1] ? parts[1] : opt;
                              const isTlSelected = selectedThinking === opt;
                              return (
                                <button
                                  key={opt}
                                  onClick={() => {
                                    setSelectedThinking(opt);
                                    // Immediately confirm with this thinking level
                                    const tl = opt;
                                    if (activeTab === "general") {
                                      api
                                        .setDefaultModel({
                                          provider_id: model.provider_id,
                                          model_id: model.id,
                                          thinking_level: tl,
                                        })
                                        .catch(console.error);
                                      onModelChange?.(model);
                                      onClose();
                                    } else {
                                      const modelStr = `${model.provider_id}/${model.id}`;
                                      api
                                        .setAgentModel({
                                          agent_type: activeTab,
                                          model_str: modelStr,
                                          thinking_level: tl,
                                        })
                                        .catch(console.error);
                                      setAgentModels((prev) => ({
                                        ...prev,
                                        [activeTab]: modelStr,
                                      }));
                                    }
                                    setExpandedModelId(null);
                                  }}
                                  className={`flex w-full items-center gap-2 px-3 py-1 text-left text-xs transition-colors hover:bg-neutral-50 dark:hover:bg-neutral-800 ${
                                    isTlSelected
                                      ? "text-amber-700 dark:text-amber-300"
                                      : "text-neutral-500 dark:text-neutral-400"
                                  }`}
                                >
                                  <span>{isTlSelected ? "●" : "○"}</span>
                                  <span>{label}</span>
                                </button>
                              );
                            })}
                          </div>
                        )}
                      </div>
                    );
                  })}{" "}
                </div>
              ),
            )
          )}
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
                  : "Using parent session model (<inherit>)"}
              </span>
            )}
          </div>
          <div className="flex items-center gap-2">
            {activeTab !== "general" && agentModels[activeTab] && (
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
