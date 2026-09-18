import { useMemo, useState } from "react";
import { Check, ChevronDown } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { Model } from "../types/api";
import { formatThinkingLevel } from "../utils/chat";
import { Button, Input, Menu } from "./ui";
import { filterModelProviderGroups, groupModelsByProvider } from "../utils/modelPicker";

interface ModelPickerProps {
  models: Model[];
  activeModel: Model | undefined;
  thinkingLevel: string | undefined;
  onSelectModel: (model: Model) => void;
  onSelectThinkingLevel: (level: string) => void;
  onOpen?: () => void;
}

export function ModelPicker({
  models,
  activeModel,
  thinkingLevel,
  onSelectModel,
  onSelectThinkingLevel,
  onOpen,
}: ModelPickerProps) {
  const { t } = useTranslation();
  const [modelSearch, setModelSearch] = useState("");
  const providers = useMemo(() => groupModelsByProvider(models), [models]);
  const filteredProviders = useMemo(
    () => filterModelProviderGroups(providers, modelSearch),
    [modelSearch, providers],
  );

  const supportsThinking = Boolean(activeModel?.thinking_levels.length);
  const selectedThinkingLevel = thinkingLevel ?? activeModel?.thinking_level;
  const currentThinking = supportsThinking
    ? formatThinkingLevel(selectedThinkingLevel ?? "")
    : t("Not available");

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
          className="composer-control model-picker-trigger"
          aria-haspopup="menu"
          variant="secondary"
          size="sm"
          trailingIcon={<ChevronDown size={13} />}
        >
          <span className="model-picker-trigger-model">
            {activeModel?.model_display_name ?? t("Select model")}
          </span>
          {supportsThinking ? (
            <span className="model-picker-trigger-thinking">{currentThinking}</span>
          ) : null}
        </Button>
      </Menu.Trigger>
      <Menu.Content className="model-picker-menu-content" side="top" align="start" sideOffset={8}>
        <Menu.Sub instant>
          <Menu.SubTrigger className="model-picker-entry">
            <span className="model-picker-entry-copy">
              <strong>{t("Model")}</strong>
              <span>{activeModel?.model_display_name ?? t("Select model")}</span>
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
            ) : (
              <div className="model-picker-empty">
                {modelSearch ? t("No models match your search") : t("No models available")}
              </div>
            )}
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
            {activeModel?.thinking_levels.map((level) => (
              <Menu.Item
                key={level}
                className={
                  selectedThinkingLevel === level
                    ? "model-picker-submenu-item selected"
                    : "model-picker-submenu-item"
                }
                onSelect={() => onSelectThinkingLevel(level)}
              >
                <span>{formatThinkingLevel(level)}</span>
                {selectedThinkingLevel === level ? <Check size={14} /> : null}
              </Menu.Item>
            ))}
          </Menu.SubContent>
        </Menu.Sub>
      </Menu.Content>
    </Menu.Root>
  );
}
