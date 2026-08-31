import { useMemo, useState } from "react";
import { Bot, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  useAgentModels,
  useModels,
  useSetAgentModel,
  useSetSubagentConfig,
  useSubagentConfig,
} from "../../hooks/workspaceQueries";
import type { Model } from "../../types/api";

const INHERIT_MODEL = "__inherit__";
const AUTOMATIC_THINKING = "__automatic__";

const AGENTS = [
  {
    type: "explorer",
    label: "Explorer",
    description: "Fast codebase search specialist",
    readOnly: true,
  },
  {
    type: "librarian",
    label: "Librarian",
    description: "Documentation and library research specialist",
    readOnly: true,
  },
  {
    type: "oracle",
    label: "Oracle",
    description: "Strategic technical advisor",
    readOnly: true,
  },
  {
    type: "fixer",
    label: "Fixer",
    description: "Implementation specialist",
    readOnly: false,
  },
] as const;

function modelKey(model: Pick<Model, "provider_id" | "model_id">): string {
  return `${model.provider_id}/${model.model_id}`;
}

function findModel(models: Model[], value: string | undefined): Model | undefined {
  if (!value) return undefined;
  return models.find(
    (model) =>
      value === modelKey(model) || value === model.model_id || value === model.model_display_name,
  );
}

function modelDisplayName(model: Model): string {
  return `${model.provider_display_name} / ${model.model_display_name}`;
}

export function AgentsSection() {
  const { t } = useTranslation();
  const {
    data: subagentConfig,
    isLoading: isLoadingConfig,
    error: configError,
  } = useSubagentConfig();
  const {
    data: agentModels,
    isLoading: isLoadingAgentModels,
    error: agentModelsError,
  } = useAgentModels();
  const { data: models = [], isLoading: isLoadingModels, error: modelsError } = useModels();
  const { mutateAsync: setAgentModel, isPending: isSavingAgentModel } = useSetAgentModel();
  const { mutateAsync: setSubagentConfig, isPending: isSavingConfig } = useSetSubagentConfig();
  const [actionError, setActionError] = useState<string | null>(null);

  const selectableModels = useMemo(() => models.filter((model) => model.connected), [models]);
  const groupedModels = useMemo(() => {
    const groups = new Map<string, { label: string; models: Model[] }>();
    for (const model of selectableModels) {
      const group = groups.get(model.provider_id);
      if (group) {
        group.models.push(model);
      } else {
        groups.set(model.provider_id, {
          label: model.provider_display_name,
          models: [model],
        });
      }
    }
    return [...groups.values()];
  }, [selectableModels]);

  const parentModel = findModel(
    models,
    agentModels?.default_model ? modelKey(agentModels.default_model) : undefined,
  );
  const isLoading = isLoadingConfig || isLoadingAgentModels || isLoadingModels;
  const loadError = configError || agentModelsError || modelsError;

  const saveAgent = async (agentType: string, modelStr: string, thinkingLevel: string) => {
    setActionError(null);
    try {
      await setAgentModel({
        agent_type: agentType,
        model_str: modelStr,
        thinking_level: thinkingLevel,
      });
    } catch (error) {
      setActionError(
        error instanceof Error ? error.message : t("Failed to save subagent settings"),
      );
    }
  };

  const handleToggle = async () => {
    if (!subagentConfig) return;
    setActionError(null);
    try {
      await setSubagentConfig({ enabled: !subagentConfig.enabled });
    } catch (error) {
      setActionError(
        error instanceof Error ? error.message : t("Failed to save subagent settings"),
      );
    }
  };

  const handleModelChange = (agentType: string, value: string) => {
    const modelStr = value === INHERIT_MODEL ? "" : value;
    void saveAgent(agentType, modelStr, "");
  };

  const handleThinkingChange = (agentType: string, value: string) => {
    const modelStr = agentModels?.agent_models[agentType] ?? "";
    const thinkingLevel = value === AUTOMATIC_THINKING ? "" : value;
    void saveAgent(agentType, modelStr, thinkingLevel);
  };

  return (
    <section className="space-y-4">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h2 className="flex items-center gap-2 text-sm font-medium text-neutral-900 dark:text-neutral-100">
            <Bot className="h-4 w-4" />
            {t("Agents")}
          </h2>
          <p className="mt-1 text-xs text-neutral-500 dark:text-neutral-400">
            {t("Configure the subagents available to the task tool")}
          </p>
        </div>
      </div>

      {isLoading ? (
        <div className="flex items-center justify-center py-12 text-sm text-neutral-500">
          <RefreshCw className="mr-2 h-4 w-4 animate-spin" />
          {t("Loading agent settings...")}
        </div>
      ) : loadError ? (
        <div className="rounded-lg border border-red-200 bg-red-50 p-3 text-xs text-red-700 dark:border-red-900/50 dark:bg-red-950/30 dark:text-red-400">
          {t("Failed to load agent settings")}
        </div>
      ) : (
        <>
          <label className="flex items-center justify-between rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
            <div>
              <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
                {t("Enable subagents")}
              </span>
              <p className="text-xs text-neutral-500 dark:text-neutral-400">
                {t("Allow the task tool to spawn subagents")}
              </p>
            </div>
            <button
              type="button"
              role="switch"
              aria-label={t("Enable subagents")}
              aria-checked={subagentConfig?.enabled ?? false}
              disabled={isSavingConfig}
              onClick={() => void handleToggle()}
              className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${
                subagentConfig?.enabled
                  ? "bg-neutral-900 dark:bg-neutral-100"
                  : "bg-neutral-300 dark:bg-neutral-600"
              }`}
            >
              <span
                className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform dark:bg-neutral-900 ${
                  subagentConfig?.enabled ? "translate-x-[18px]" : "translate-x-[2px]"
                }`}
              />
            </button>
          </label>

          <div className="rounded-lg border border-neutral-200 dark:border-neutral-800">
            <div className="border-b border-neutral-200 px-3 py-2 dark:border-neutral-800">
              <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
                {t("Subagent models")}
              </h3>
              <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
                {t("Choose a model for each subagent or inherit the main agent model")}
              </p>
            </div>

            <div className="divide-y divide-neutral-200 dark:divide-neutral-800">
              {AGENTS.map((agent) => {
                const configuredModel = agentModels?.agent_models[agent.type];
                const configuredModelInfo = findModel(models, configuredModel);
                const selectedModel = configuredModelInfo ?? parentModel;
                const configuredThinking =
                  agentModels?.agent_thinking_levels?.[agent.type] ?? AUTOMATIC_THINKING;
                const thinkingOptions = selectedModel?.thinking_levels ?? [];
                const thinkingValue = thinkingOptions.includes(configuredThinking)
                  ? configuredThinking
                  : AUTOMATIC_THINKING;
                const hasUnavailableModel =
                  configuredModel !== undefined &&
                  (!configuredModelInfo || !configuredModelInfo.connected);
                const modelSelectValue = configuredModel
                  ? configuredModelInfo
                    ? modelKey(configuredModelInfo)
                    : configuredModel
                  : INHERIT_MODEL;

                return (
                  <div key={agent.type} className="space-y-2 p-3">
                    <div className="flex flex-col gap-1 sm:flex-row sm:items-center sm:justify-between">
                      <div>
                        <div className="flex items-center gap-2">
                          <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
                            {t(agent.label)}
                          </span>
                          <span className="rounded-full bg-neutral-100 px-1.5 py-0.5 text-[10px] text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400">
                            {agent.readOnly ? t("Read-only") : t("Build")}
                          </span>
                        </div>
                        <p className="text-xs text-neutral-500 dark:text-neutral-400">
                          {t(agent.description)}
                        </p>
                      </div>
                      {selectedModel && (
                        <span className="text-xs text-neutral-400 dark:text-neutral-500">
                          {modelDisplayName(selectedModel)}
                        </span>
                      )}
                    </div>

                    <div className="grid gap-2 sm:grid-cols-2">
                      <label className="flex flex-col gap-1">
                        <span className="text-xs text-neutral-500 dark:text-neutral-400">
                          {t("Model")}
                        </span>
                        <select
                          value={modelSelectValue}
                          disabled={isSavingAgentModel}
                          onChange={(event) => handleModelChange(agent.type, event.target.value)}
                          className="w-full rounded-md border border-neutral-300 bg-white px-2.5 py-1.5 text-sm text-neutral-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 disabled:cursor-not-allowed disabled:opacity-60 dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-100"
                        >
                          <option value={INHERIT_MODEL}>{t("Inherit main agent model")}</option>
                          {hasUnavailableModel && (
                            <option value={modelSelectValue}>
                              {configuredModelInfo
                                ? modelDisplayName(configuredModelInfo)
                                : configuredModel}{" "}
                              ({t("Unavailable")})
                            </option>
                          )}
                          {groupedModels.map((group) => (
                            <optgroup key={group.label} label={group.label}>
                              {group.models.map((model) => (
                                <option key={modelKey(model)} value={modelKey(model)}>
                                  {model.model_display_name}
                                </option>
                              ))}
                            </optgroup>
                          ))}
                        </select>
                      </label>

                      <label className="flex flex-col gap-1">
                        <span className="text-xs text-neutral-500 dark:text-neutral-400">
                          {t("Thinking level")}
                        </span>
                        <select
                          value={thinkingValue}
                          disabled={thinkingOptions.length === 0 || isSavingAgentModel}
                          onChange={(event) => handleThinkingChange(agent.type, event.target.value)}
                          className="w-full rounded-md border border-neutral-300 bg-white px-2.5 py-1.5 text-sm text-neutral-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 disabled:cursor-not-allowed disabled:opacity-60 dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-100"
                        >
                          <option value={AUTOMATIC_THINKING}>{t("Automatic")}</option>
                          {thinkingOptions.map((level) => (
                            <option key={level} value={level}>
                              {level}
                            </option>
                          ))}
                        </select>
                      </label>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        </>
      )}

      {actionError && (
        <p className="text-xs text-red-600 dark:text-red-400" role="alert">
          {actionError}
        </p>
      )}
    </section>
  );
}
