import { useState } from "react";
import { RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  useAgentModels,
  useModels,
  useSetAgentModel,
  useSetSubagentConfig,
  useSubagentConfig,
} from "../../hooks/workspaceQueries";
import type { Model } from "../../types/api";
import { ModelPicker } from "../ModelPicker";
import {
  SettingsSectionHeader,
  SettingsGroup,
  SettingsSwitchRow,
  SettingsRow,
} from "./SettingsCommon";

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
    <section className="space-y-6">
      <SettingsSectionHeader
        title={t("Agents")}
        description={t("Configure the subagents available to the task tool")}
      />

      {isLoading ? (
        <div className="flex items-center justify-center py-12 text-sm text-neutral-500">
          <RefreshCw className="mr-2 h-4 w-4 animate-spin" />
          {t("Loading agent settings...")}
        </div>
      ) : loadError ? (
        <div className="rounded-xl border border-red-200 bg-red-50 p-4 text-xs text-red-700 dark:border-red-900/50 dark:bg-red-950/30 dark:text-red-400">
          {t("Failed to load agent settings")}
        </div>
      ) : (
        <>
          <SettingsGroup title={t("Subagent Capabilities")}>
            <SettingsSwitchRow
              label={t("Enable subagents")}
              description={t("Allow the task tool to spawn subagents")}
              checked={subagentConfig?.enabled ?? false}
              disabled={isSavingConfig}
              onCheckedChange={() => void handleToggle()}
            />
          </SettingsGroup>

          <SettingsGroup title={t("Subagent models")}>
            {AGENTS.map((agent) => {
              const configuredModel = agentModels?.agent_models[agent.type];
              const configuredModelInfo = findModel(models, configuredModel);
              const configuredThinking = agentModels?.agent_thinking_levels?.[agent.type] ?? "";
              const isInherited = !configuredModel || configuredModel === INHERIT_MODEL;
              const isAutomatic = !configuredThinking || configuredThinking === AUTOMATIC_THINKING;
              const hasUnavailableModel =
                Boolean(configuredModel) &&
                !isInherited &&
                (!configuredModelInfo || !configuredModelInfo.connected);
              const unavailableLabel = hasUnavailableModel
                ? `${configuredModelInfo ? modelDisplayName(configuredModelInfo) : configuredModel} (${t("Unavailable")})`
                : undefined;

              return (
                <SettingsRow
                  key={agent.type}
                  label={t(agent.label)}
                  badge={
                    <span className="rounded-full bg-neutral-200/70 dark:bg-neutral-700/70 px-2 py-0.5 text-[10px] font-medium text-neutral-700 dark:text-neutral-300">
                      {agent.readOnly ? t("Read-only") : t("Build")}
                    </span>
                  }
                  description={t(agent.description)}
                  control={
                    <ModelPicker
                      models={models}
                      activeModel={configuredModelInfo}
                      parentModel={parentModel}
                      isInherited={isInherited}
                      allowInherit
                      onSelectInherit={() => handleModelChange(agent.type, INHERIT_MODEL)}
                      onSelectModel={(model) => handleModelChange(agent.type, modelKey(model))}
                      thinkingLevel={configuredThinking}
                      allowAutomaticThinking
                      isAutomaticThinking={isAutomatic}
                      onSelectThinkingLevel={(level) => handleThinkingChange(agent.type, level)}
                      unavailableModelLabel={unavailableLabel}
                      disabled={isSavingAgentModel}
                      side="bottom"
                      align="end"
                      triggerClassName="settings-model-picker-trigger"
                      ariaLabel={`${t(agent.label)} ${t("Model")}`}
                    />
                  }
                />
              );
            })}
          </SettingsGroup>
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
