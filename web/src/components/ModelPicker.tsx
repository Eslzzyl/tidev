import { useMemo } from "react";
import { Check, ChevronDown } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { Model } from "../types/api";
import { formatThinkingLevel } from "../utils/chat";
import { Button, Menu } from "./ui";

interface ModelPickerProps {
  models: Model[];
  activeModel: Model | undefined;
  thinkingLevel: string | undefined;
  onSelectModel: (model: Model) => void;
  onSelectThinkingLevel: (level: string) => void;
  onOpen?: () => void;
}

interface ProviderGroup {
  id: string;
  name: string;
  connected: boolean;
  models: Model[];
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
  const providers = useMemo<ProviderGroup[]>(() => {
    const groups = new Map<string, ProviderGroup>();
    for (const model of models) {
      const group = groups.get(model.provider_id);
      if (group) {
        group.models.push(model);
        group.connected ||= model.connected;
      } else {
        groups.set(model.provider_id, {
          id: model.provider_id,
          name: model.provider_display_name,
          connected: model.connected,
          models: [model],
        });
      }
    }
    return [...groups.values()].sort(
      (left, right) => Number(right.connected) - Number(left.connected),
    );
  }, [models]);

  const supportsThinking = Boolean(activeModel?.thinking_levels.length);
  const selectedThinkingLevel = thinkingLevel ?? activeModel?.thinking_level;
  const currentThinking = supportsThinking
    ? formatThinkingLevel(selectedThinkingLevel ?? "")
    : t("Not available");

  return (
    <Menu.Root onOpenChange={(nextOpen) => nextOpen && onOpen?.()}>
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
            {providers.length ? (
              providers.map((provider) =>
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
              <div className="model-picker-empty">{t("No models available")}</div>
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
