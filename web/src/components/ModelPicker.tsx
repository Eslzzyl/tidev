import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Check, ChevronDown, ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { Model } from "../types/api";
import { formatThinkingLevel } from "../utils/chat";

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

type Submenu = "providers" | "thinking" | null;

export function ModelPicker({
  models,
  activeModel,
  thinkingLevel,
  onSelectModel,
  onSelectThinkingLevel,
  onOpen,
}: ModelPickerProps) {
  const { t } = useTranslation();
  const pickerRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [submenu, setSubmenu] = useState<Submenu>(null);
  const [providerId, setProviderId] = useState<string | null>(null);

  const providers = useMemo(() => {
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

  const selectedProvider = providers.find((provider) => provider.id === providerId);
  const supportsThinking = Boolean(activeModel?.thinking_levels.length);
  const selectedThinkingLevel = thinkingLevel ?? activeModel?.thinking_level;
  const currentThinking = supportsThinking
    ? formatThinkingLevel(selectedThinkingLevel ?? "")
    : t("Not available");

  const clearSubmenu = useCallback(() => {
    setSubmenu(null);
    setProviderId(null);
  }, []);

  const closePicker = useCallback(() => {
    setOpen(false);
    clearSubmenu();
  }, [clearSubmenu]);

  useEffect(() => {
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node) || !pickerRef.current?.contains(target)) {
        closePicker();
      }
    };

    document.addEventListener("pointerdown", handlePointerDown);
    return () => document.removeEventListener("pointerdown", handlePointerDown);
  }, [closePicker]);

  const handleTriggerClick = () => {
    onOpen?.();
    setOpen((current) => !current);
    setSubmenu(null);
    setProviderId(null);
  };

  const handleSelectModel = (model: Model) => onSelectModel(model);

  return (
    <div ref={pickerRef} className="composer-menu model-picker">
      <button
        type="button"
        className="composer-control neutral model-picker-trigger"
        onClick={handleTriggerClick}
        aria-expanded={open}
        aria-haspopup="menu"
      >
        <span className="model-picker-trigger-model">
          {activeModel?.model_display_name ?? t("Select model")}
        </span>
        {supportsThinking ? (
          <span className="model-picker-trigger-thinking">{currentThinking}</span>
        ) : null}
        <ChevronDown size={13} />
      </button>
      {open ? (
        <div
          className={
            submenu
              ? "composer-popover model-picker-popover has-submenu"
              : "composer-popover model-picker-popover"
          }
          role="menu"
          onMouseLeave={clearSubmenu}
        >
          <div className="model-picker-root model-picker-panel">
            <button
              type="button"
              className={
                submenu === "providers" ? "model-picker-entry selected" : "model-picker-entry"
              }
              onClick={() => {
                setSubmenu(submenu === "providers" ? null : "providers");
                setProviderId(null);
              }}
              onMouseEnter={() => {
                setSubmenu("providers");
                setProviderId(null);
              }}
              aria-expanded={submenu === "providers"}
              aria-haspopup="menu"
            >
              <span className="model-picker-entry-copy">
                <strong>{t("Model")}</strong>
                <span>{activeModel?.model_display_name ?? t("Select model")}</span>
              </span>
              <ChevronRight size={15} />
            </button>
            <button
              type="button"
              className={
                supportsThinking && submenu === "thinking"
                  ? "model-picker-entry selected"
                  : "model-picker-entry"
              }
              disabled={!supportsThinking}
              onClick={() => {
                if (!supportsThinking) return;
                setSubmenu(submenu === "thinking" ? null : "thinking");
                setProviderId(null);
              }}
              onMouseEnter={() => {
                if (!supportsThinking) return;
                setSubmenu("thinking");
                setProviderId(null);
              }}
              aria-disabled={!supportsThinking}
              aria-expanded={supportsThinking && submenu === "thinking"}
              aria-haspopup="menu"
            >
              <span className="model-picker-entry-copy">
                <strong>{t("Thinking level")}</strong>
                <span>{currentThinking}</span>
              </span>
              {supportsThinking ? <ChevronRight size={15} /> : null}
            </button>
          </div>

          {submenu === "providers" ? (
            <div
              className={
                selectedProvider?.connected
                  ? "model-picker-submenus has-selected-provider"
                  : "model-picker-submenus"
              }
            >
              <div
                className="model-picker-submenu provider-picker-submenu model-picker-panel"
                role="menu"
              >
                {providers.length ? (
                  providers.map((provider) => (
                    <div
                      key={provider.id}
                      className="model-picker-provider-item"
                      onMouseEnter={() => setProviderId(provider.connected ? provider.id : null)}
                    >
                      <button
                        type="button"
                        className={
                          provider.id === providerId && provider.connected
                            ? "model-picker-submenu-item selected"
                            : provider.connected
                              ? "model-picker-submenu-item"
                              : "model-picker-submenu-item provider-disabled"
                        }
                        disabled={!provider.connected}
                        onClick={() => {
                          if (provider.connected) setProviderId(provider.id);
                        }}
                        aria-expanded={provider.connected && provider.id === providerId}
                        aria-haspopup="menu"
                      >
                        <span>{provider.name}</span>
                        {provider.connected ? <ChevronRight size={15} /> : null}
                      </button>
                    </div>
                  ))
                ) : (
                  <div className="model-picker-empty">{t("No models available")}</div>
                )}
              </div>

              {selectedProvider?.connected ? (
                <div
                  className="model-picker-submenu model-picker-models model-picker-panel"
                  role="menu"
                >
                  {selectedProvider.models.map((model) => {
                    const selected =
                      activeModel?.provider_id === model.provider_id &&
                      activeModel.model_id === model.model_id;
                    return (
                      <button
                        type="button"
                        key={`${model.provider_id}:${model.model_id}`}
                        className={
                          selected
                            ? "model-picker-submenu-item model-picker-model selected"
                            : "model-picker-submenu-item model-picker-model"
                        }
                        disabled={!model.connected}
                        onClick={() => handleSelectModel(model)}
                      >
                        <span>{model.model_display_name}</span>
                        {selected ? <Check size={14} /> : null}
                        {!model.connected ? <small>{t("Not connected")}</small> : null}
                      </button>
                    );
                  })}
                </div>
              ) : null}
            </div>
          ) : submenu === "thinking" ? (
            <div className="model-picker-submenus">
              <div
                className="model-picker-submenu thinking-picker-submenu model-picker-panel"
                role="menu"
              >
                {activeModel?.thinking_levels.map((level) => (
                  <button
                    type="button"
                    key={level}
                    className={
                      selectedThinkingLevel === level
                        ? "model-picker-submenu-item selected"
                        : "model-picker-submenu-item"
                    }
                    onClick={() => {
                      onSelectThinkingLevel(level);
                    }}
                  >
                    <span>{formatThinkingLevel(level)}</span>
                    {selectedThinkingLevel === level ? <Check size={14} /> : null}
                  </button>
                ))}
              </div>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
