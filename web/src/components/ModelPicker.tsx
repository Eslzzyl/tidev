import { useMemo, useState } from "react";
import { Check, ChevronDown, Zap } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { Model } from "../types/api";
import { formatThinkingLevel } from "../utils/chat";
import { Button, Input, Menu } from "./ui";
import { cx } from "./ui/utils";
import { filterModelProviderGroups, groupModelsByProvider } from "../utils/modelPicker";

export interface ModelPickerProps {
  models: Model[];
  activeModel: Model | undefined;
  thinkingLevel: string | undefined;
  fastMode?: boolean;
  onSelectModel: (model: Model) => void;
  onSelectThinkingLevel: (level: string) => void;
  onToggleFastMode?: () => void;
  onOpen?: () => void;

  // Subagent / inherit mode options
  allowInherit?: boolean;
  isInherited?: boolean;
  parentModel?: Model;
  onSelectInherit?: () => void;
  unavailableModelLabel?: string;

  // Thinking level options
  allowAutomaticThinking?: boolean;
  isAutomaticThinking?: boolean;

  // Visual & placement options
  disabled?: boolean;
  side?: "top" | "bottom" | "left" | "right";
  align?: "start" | "end" | "center";
  sideOffset?: number;
  triggerVariant?: "secondary" | "ghost";
  triggerClassName?: string;
  ariaLabel?: string;
}

export function ModelPicker({
  models,
  activeModel,
  thinkingLevel,
  fastMode = false,
  onSelectModel,
  onSelectThinkingLevel,
  onToggleFastMode,
  onOpen,
  allowInherit = false,
  isInherited = false,
  parentModel,
  onSelectInherit,
  unavailableModelLabel,
  allowAutomaticThinking = false,
  isAutomaticThinking = false,
  disabled = false,
  side = "top",
  align = "start",
  sideOffset = 8,
  triggerVariant = "secondary",
  triggerClassName,
  ariaLabel,
}: ModelPickerProps) {
  const { t } = useTranslation();
  const [modelSearch, setModelSearch] = useState("");
  const providers = useMemo(() => groupModelsByProvider(models), [models]);
  const filteredProviders = useMemo(
    () => filterModelProviderGroups(providers, modelSearch),
    [modelSearch, providers],
  );

  const effectiveModel = isInherited ? parentModel : activeModel;
  const supportsThinking = Boolean(effectiveModel?.thinking_levels.length);
  const supportsFastMode = Boolean(onToggleFastMode && effectiveModel?.is_gpt);
  const selectedThinkingLevel = thinkingLevel ?? effectiveModel?.thinking_level;

  let currentThinking: string;
  if (!supportsThinking) {
    currentThinking = t("Not available");
  } else if (allowAutomaticThinking && (isAutomaticThinking || !selectedThinkingLevel)) {
    currentThinking = t("Automatic");
  } else {
    currentThinking = formatThinkingLevel(selectedThinkingLevel ?? "");
  }

  let triggerModelLabel: string;
  if (isInherited) {
    triggerModelLabel = parentModel
      ? `${t("Inherit main agent model")} (${parentModel.model_display_name})`
      : t("Inherit main agent model");
  } else if (activeModel) {
    triggerModelLabel = activeModel.model_display_name;
  } else if (unavailableModelLabel) {
    triggerModelLabel = unavailableModelLabel;
  } else {
    triggerModelLabel = t("Select model");
  }

  const inheritLabel = parentModel
    ? `${t("Inherit main agent model")} (${parentModel.model_display_name})`
    : t("Inherit main agent model");

  const normalizedQuery = modelSearch.trim().toLowerCase();
  const showInheritInSearch =
    allowInherit &&
    (!normalizedQuery ||
      inheritLabel.toLowerCase().includes(normalizedQuery) ||
      t("Inherit main agent model").toLowerCase().includes(normalizedQuery) ||
      "inherit".includes(normalizedQuery));

  return (
    <Menu.Root
      onOpenChange={(nextOpen) => {
        if (nextOpen) {
          setModelSearch("");
          onOpen?.();
        } else {
          setModelSearch("");
        }
      }}
    >
      <Menu.Trigger asChild>
        <Button
          type="button"
          className={cx("composer-control model-picker-trigger", triggerClassName)}
          aria-haspopup="menu"
          aria-label={ariaLabel}
          variant={triggerVariant}
          size="sm"
          disabled={disabled}
          trailingIcon={<ChevronDown size={13} />}
        >
          {supportsFastMode && fastMode ? (
            <span
              className="model-picker-trigger-thinking model-picker-trigger-fast"
              title={t("Fast")}
            >
              <Zap size={12} aria-label={t("Fast")} role="img" />
            </span>
          ) : null}
          <span className="model-picker-trigger-model">{triggerModelLabel}</span>
          {supportsThinking ? (
            <span className="model-picker-trigger-thinking">{currentThinking}</span>
          ) : null}
        </Button>
      </Menu.Trigger>
      <Menu.Content
        className="model-picker-menu-content"
        side={side}
        align={align}
        sideOffset={sideOffset}
      >
        <Menu.Sub instant>
          <Menu.SubTrigger className="model-picker-entry">
            <span className="model-picker-entry-copy">
              <strong>{t("Model")}</strong>
              <span>{triggerModelLabel}</span>
            </span>
          </Menu.SubTrigger>
          <Menu.SubContent className="model-picker-submenu model-picker-panel">
            <Input
              className="model-picker-search-input"
              type="search"
              value={modelSearch}
              aria-label={t("Search models")}
              placeholder={t("Search models...")}
              onChange={(event) => setModelSearch(event.target.value)}
              onKeyDown={(event) => {
                if (
                  event.key !== "Escape" &&
                  event.key !== "ArrowDown" &&
                  event.key !== "ArrowUp" &&
                  event.key !== "Tab"
                ) {
                  event.stopPropagation();
                }
              }}
            />
            {showInheritInSearch ? (
              <>
                <Menu.Item
                  className={
                    isInherited
                      ? "model-picker-submenu-item model-picker-model selected"
                      : "model-picker-submenu-item model-picker-model"
                  }
                  onSelect={() => onSelectInherit?.()}
                >
                  <span>{inheritLabel}</span>
                  {isInherited ? <Check size={14} /> : null}
                </Menu.Item>
                <Menu.Separator />
              </>
            ) : null}
            {unavailableModelLabel && !isInherited && !activeModel ? (
              <>
                <Menu.Item
                  disabled
                  className="model-picker-submenu-item model-picker-model selected"
                >
                  <span>{unavailableModelLabel}</span>
                  <Check size={14} />
                </Menu.Item>
                <Menu.Separator />
              </>
            ) : null}
            {filteredProviders.length ? (
              filteredProviders.map((provider) =>
                provider.connected ? (
                  <Menu.Sub key={provider.id} instant>
                    <Menu.SubTrigger className="model-picker-submenu-item">
                      <span>{provider.name}</span>
                    </Menu.SubTrigger>
                    <Menu.SubContent className="model-picker-submenu model-picker-models model-picker-panel">
                      {provider.models.map((model) => {
                        const selected =
                          !isInherited &&
                          activeModel?.provider_id === model.provider_id &&
                          activeModel.model_id === model.model_id;
                        return (
                          <Menu.Item
                            key={`${model.provider_id}:${model.model_id}`}
                            disabled={!model.connected}
                            className={
                              selected
                                ? "model-picker-submenu-item model-picker-model selected"
                                : "model-picker-submenu-item model-picker-model"
                            }
                            onSelect={() => onSelectModel(model)}
                          >
                            <span>{model.model_display_name}</span>
                            {selected ? <Check size={14} /> : null}
                            {!model.connected ? <small>{t("Not connected")}</small> : null}
                          </Menu.Item>
                        );
                      })}
                    </Menu.SubContent>
                  </Menu.Sub>
                ) : (
                  <Menu.Item
                    key={provider.id}
                    disabled
                    className="model-picker-submenu-item provider-disabled"
                  >
                    <span>{provider.name}</span>
                    <small>{t("Not connected")}</small>
                  </Menu.Item>
                ),
              )
            ) : !showInheritInSearch ? (
              <div className="model-picker-empty">
                {modelSearch ? t("No models match your search") : t("No models available")}
              </div>
            ) : null}
          </Menu.SubContent>
        </Menu.Sub>

        <Menu.Sub instant>
          <Menu.SubTrigger className="model-picker-entry" disabled={!supportsThinking}>
            <span className="model-picker-entry-copy">
              <strong>{t("Thinking level")}</strong>
              <span>{currentThinking}</span>
            </span>
          </Menu.SubTrigger>
          <Menu.SubContent className="model-picker-submenu thinking-picker-submenu model-picker-panel">
            {allowAutomaticThinking ? (
              <Menu.Item
                className={
                  isAutomaticThinking || !selectedThinkingLevel
                    ? "model-picker-submenu-item selected"
                    : "model-picker-submenu-item"
                }
                onSelect={() => onSelectThinkingLevel("")}
              >
                <span>{t("Automatic")}</span>
                {isAutomaticThinking || !selectedThinkingLevel ? <Check size={14} /> : null}
              </Menu.Item>
            ) : null}
            {effectiveModel?.thinking_levels.map((level) => {
              const selected = !isAutomaticThinking && selectedThinkingLevel === level;
              return (
                <Menu.Item
                  key={level}
                  className={
                    selected ? "model-picker-submenu-item selected" : "model-picker-submenu-item"
                  }
                  onSelect={() => onSelectThinkingLevel(level)}
                >
                  <span>{formatThinkingLevel(level)}</span>
                  {selected ? <Check size={14} /> : null}
                </Menu.Item>
              );
            })}
          </Menu.SubContent>
        </Menu.Sub>

        {supportsFastMode ? (
          <>
            <Menu.Separator />
            <Menu.Item
              className={
                fastMode
                  ? "model-picker-entry model-picker-fast-mode selected"
                  : "model-picker-entry model-picker-fast-mode"
              }
              onSelect={() => onToggleFastMode?.()}
            >
              <span className="model-picker-entry-copy">
                <strong>{t("Fast mode")}</strong>
              </span>
              {fastMode ? <Check size={14} /> : null}
            </Menu.Item>
          </>
        ) : null}
      </Menu.Content>
    </Menu.Root>
  );
}
