import { useState } from "react";
import { KeyRound, Plus, Server, Trash2, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  useConnectProvider,
  useCreateProvider,
  useDeleteProvider,
  useDisconnectProvider,
  useProviders,
} from "../../hooks/workspaceQueries";
import type { CreateModelRequest, CreateProviderRequest, ProviderInfo } from "../../types/api";
import { Button, Dialog, Field, IconButton, Input, Select } from "../ui";
import { ConfirmDialog } from "../ui/ConfirmDialog";

const API_TYPE_OPTIONS = [
  { value: "openai_chat_completions", label: "OpenAI Chat Completions" },
  { value: "openai_responses", label: "OpenAI Responses" },
  { value: "anthropic", label: "Anthropic Messages" },
  { value: "google_gemini", label: "Google Gemini" },
];

interface ModelDraft {
  model_id: string;
  display_name: string;
  request_model_id: string;
  context_window: string;
  max_output_tokens: string;
  temperature: string;
  supports_images: boolean;
}

interface ProviderDraft {
  provider_id: string;
  display_name: string;
  base_url: string;
  api_type: string;
  api_key: string;
  models: ModelDraft[];
}

const EMPTY_MODEL: ModelDraft = {
  model_id: "",
  display_name: "",
  request_model_id: "",
  context_window: "128000",
  max_output_tokens: "16384",
  temperature: "0.7",
  supports_images: false,
};

const EMPTY_PROVIDER: ProviderDraft = {
  provider_id: "",
  display_name: "",
  base_url: "",
  api_type: "openai_chat_completions",
  api_key: "",
  models: [{ ...EMPTY_MODEL }],
};

function providerDraftToRequest(draft: ProviderDraft): CreateProviderRequest {
  return {
    provider_id: draft.provider_id.trim(),
    display_name: draft.display_name.trim(),
    base_url: draft.base_url.trim(),
    api_type: draft.api_type,
    api_key: draft.api_key,
    models: draft.models.map((model): CreateModelRequest => {
      const request: CreateModelRequest = {
        model_id: model.model_id.trim(),
        display_name: model.display_name.trim(),
        context_window: Number(model.context_window),
        max_output_tokens: Number(model.max_output_tokens),
        temperature: Number(model.temperature),
        supports_images: model.supports_images,
      };
      if (model.request_model_id.trim()) request.request_model_id = model.request_model_id.trim();
      return request;
    }),
  };
}

function validateProviderDraft(draft: ProviderDraft, t: (key: string) => string): string | null {
  if (!draft.provider_id.trim()) return t("Provider ID is required");
  if (!draft.display_name.trim()) return t("Provider name is required");
  if (!draft.base_url.trim()) return t("Base URL is required");
  if (!draft.api_key.trim()) return t("API key is required");
  if (draft.models.length === 0) return t("At least one model is required");

  for (const model of draft.models) {
    if (!model.model_id.trim() || !model.display_name.trim()) {
      return t("Every model needs an ID and display name");
    }
    if (Number(model.context_window) <= 0 || Number(model.max_output_tokens) <= 0) {
      return t("Model token limits must be greater than zero");
    }
    if (!Number.isFinite(Number(model.temperature))) {
      return t("Model temperature must be a valid number");
    }
  }
  return null;
}

export function ProvidersSection() {
  const { t } = useTranslation();
  const { data: providers = [], isLoading, error } = useProviders();
  const { mutateAsync: connectProvider, isPending: isConnecting } = useConnectProvider();
  const { mutateAsync: disconnectProvider, isPending: isDisconnecting } = useDisconnectProvider();
  const { mutateAsync: createProvider, isPending: isCreating } = useCreateProvider();
  const { mutateAsync: deleteProvider, isPending: isDeleting } = useDeleteProvider();

  const [connectTarget, setConnectTarget] = useState<ProviderInfo | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [connectError, setConnectError] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [draft, setDraft] = useState<ProviderDraft>({
    ...EMPTY_PROVIDER,
    models: [{ ...EMPTY_MODEL }],
  });
  const [activeModelIndex, setActiveModelIndex] = useState(0);
  const [formError, setFormError] = useState<string | null>(null);
  const [providerToDelete, setProviderToDelete] = useState<ProviderInfo | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const openConnect = (provider: ProviderInfo) => {
    setConnectTarget(provider);
    setApiKey("");
    setConnectError(null);
    setActionError(null);
  };

  const closeConnect = () => {
    if (isConnecting) return;
    setConnectTarget(null);
    setApiKey("");
    setConnectError(null);
  };

  const handleConnect = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!connectTarget) return;
    if (!apiKey.trim()) {
      setConnectError(t("API key is required"));
      return;
    }

    setConnectError(null);
    try {
      await connectProvider({ id: connectTarget.id, data: { api_key: apiKey } });
      setConnectTarget(null);
      setApiKey("");
    } catch (reason) {
      setConnectError(reason instanceof Error ? reason.message : t("Failed to save API key"));
    }
  };

  const openAdd = () => {
    setDraft({ ...EMPTY_PROVIDER, models: [{ ...EMPTY_MODEL }] });
    setActiveModelIndex(0);
    setFormError(null);
    setActionError(null);
    setAddOpen(true);
  };

  const closeAdd = () => {
    if (isCreating) return;
    setAddOpen(false);
    setFormError(null);
  };

  const handleCreate = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const validationError = validateProviderDraft(draft, t);
    if (validationError) {
      setFormError(validationError);
      return;
    }

    setFormError(null);
    try {
      await createProvider(providerDraftToRequest(draft));
      setAddOpen(false);
    } catch (reason) {
      setFormError(reason instanceof Error ? reason.message : t("Failed to create provider"));
    }
  };

  const handleDisconnect = async (provider: ProviderInfo) => {
    setActionError(null);
    try {
      await disconnectProvider(provider.id);
    } catch (reason) {
      setActionError(reason instanceof Error ? reason.message : t("Failed to disconnect provider"));
    }
  };

  const handleDelete = async () => {
    if (!providerToDelete) return;
    const provider = providerToDelete;
    setActionError(null);
    try {
      await deleteProvider(provider.id);
      setProviderToDelete(null);
    } catch (reason) {
      setProviderToDelete(null);
      setActionError(reason instanceof Error ? reason.message : t("Failed to delete provider"));
    }
  };

  const updateModel = (index: number, partial: Partial<ModelDraft>) => {
    setDraft((current) => ({
      ...current,
      models: current.models.map((model, modelIndex) =>
        modelIndex === index ? { ...model, ...partial } : model,
      ),
    }));
  };

  const addModel = () => {
    const nextIndex = draft.models.length;
    setDraft((current) => ({
      ...current,
      models: [...current.models, { ...EMPTY_MODEL }],
    }));
    setActiveModelIndex(nextIndex);
  };

  const removeModel = (index: number) => {
    const nextIndex =
      activeModelIndex > index
        ? activeModelIndex - 1
        : Math.min(activeModelIndex, draft.models.length - 2);
    setDraft((current) => ({
      ...current,
      models: current.models.filter((_, modelIndex) => modelIndex !== index),
    }));
    setActiveModelIndex(nextIndex);
  };

  const activeModel = draft.models[activeModelIndex];

  return (
    <section className="space-y-4">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
            {t("Providers")}
          </h2>
          <p className="text-xs text-neutral-500 dark:text-neutral-400">
            {t("Manage provider API keys and custom model endpoints")}
          </p>
        </div>
        <Button
          type="button"
          variant="primary"
          size="sm"
          leadingIcon={<Plus className="h-3.5 w-3.5" />}
          onClick={openAdd}
        >
          {t("Add Provider")}
        </Button>
      </div>

      {actionError ? (
        <div
          className="rounded-lg bg-red-50 p-2.5 text-xs text-red-700 dark:bg-red-950/40 dark:text-red-300"
          role="alert"
        >
          {actionError}
        </div>
      ) : null}

      {isLoading ? (
        <div className="flex items-center justify-center rounded-xl border border-dashed border-neutral-300 py-12 text-sm text-neutral-500 dark:border-neutral-800 dark:text-neutral-400">
          {t("Loading providers...")}
        </div>
      ) : error ? (
        <div
          className="rounded-lg border border-red-200 bg-red-50 p-3 text-xs text-red-700 dark:border-red-900/50 dark:bg-red-950/30 dark:text-red-400"
          role="alert"
        >
          {t("Failed to load providers")}
        </div>
      ) : providers.length === 0 ? (
        <div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-neutral-300 py-12 text-center dark:border-neutral-800">
          <Server className="mb-2 h-10 w-10 text-neutral-400" />
          <p className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
            {t("No providers configured")}
          </p>
          <Button type="button" className="mt-4" variant="primary" size="sm" onClick={openAdd}>
            {t("Add your first provider")}
          </Button>
        </div>
      ) : (
        <div className="space-y-3">
          {providers.map((provider) => {
            const isBundled = provider.source === "bundled";
            const isBusy =
              (isConnecting && connectTarget?.id === provider.id) ||
              isDisconnecting ||
              (isDeleting && providerToDelete?.id === provider.id);

            return (
              <div
                key={provider.id}
                className="overflow-hidden rounded-xl border border-neutral-200 bg-white shadow-sm dark:border-neutral-800 dark:bg-neutral-900"
              >
                <div className="flex flex-wrap items-start justify-between gap-3 p-3.5">
                  <div className="flex min-w-0 items-start gap-3">
                    <span
                      className={`mt-1.5 h-2.5 w-2.5 shrink-0 rounded-full ${
                        provider.connected ? "bg-emerald-500" : "bg-neutral-400"
                      }`}
                      aria-label={provider.connected ? t("Connected") : t("Not connected")}
                    />
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="truncate text-sm font-semibold text-neutral-900 dark:text-neutral-100">
                          {provider.display_name}
                        </span>
                        <span className="font-mono text-[10px] text-neutral-500 dark:text-neutral-400">
                          {provider.id}
                        </span>
                        <span
                          className={`inline-flex rounded-md px-1.5 py-0.5 text-[10px] font-medium ${
                            isBundled
                              ? "border border-indigo-200/60 bg-indigo-50 text-indigo-700 dark:border-indigo-800/60 dark:bg-indigo-950/50 dark:text-indigo-300"
                              : "border border-emerald-200/60 bg-emerald-50 text-emerald-700 dark:border-emerald-800/60 dark:bg-emerald-950/50 dark:text-emerald-300"
                          }`}
                        >
                          {isBundled ? t("Preset") : t("Custom")}
                        </span>
                      </div>
                      <p className="mt-1 truncate font-mono text-xs text-neutral-500 dark:text-neutral-400">
                        {provider.base_url}
                      </p>
                      <p className="mt-1 text-[11px] text-neutral-500 dark:text-neutral-400">
                        {t("{{count}} models", { count: provider.models.length })} ·{" "}
                        {provider.connected ? t("API key configured") : t("API key not configured")}
                      </p>
                    </div>
                  </div>

                  <div className="flex shrink-0 flex-wrap items-center justify-end gap-1.5">
                    <Button
                      type="button"
                      size="sm"
                      variant={provider.connected ? "secondary" : "primary"}
                      leadingIcon={<KeyRound className="h-3.5 w-3.5" />}
                      onClick={() => openConnect(provider)}
                      disabled={isBusy}
                    >
                      {provider.connected ? t("Update key") : t("Configure key")}
                    </Button>
                    {provider.connected ? (
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        onClick={() => void handleDisconnect(provider)}
                        disabled={isBusy}
                      >
                        {t("Disconnect")}
                      </Button>
                    ) : null}
                    {provider.can_delete ? (
                      <IconButton
                        type="button"
                        size="sm"
                        variant="danger"
                        label={t("Delete provider")}
                        onClick={() => setProviderToDelete(provider)}
                        disabled={isBusy}
                      >
                        <Trash2 className="h-4 w-4" />
                      </IconButton>
                    ) : null}
                  </div>
                </div>

                {provider.models.length > 0 ? (
                  <details className="border-t border-neutral-200 dark:border-neutral-800">
                    <summary className="cursor-pointer px-3.5 py-2 text-xs font-medium text-neutral-600 dark:text-neutral-400">
                      {t("View models")}
                    </summary>
                    <div className="grid gap-2 border-t border-neutral-200 bg-neutral-50/50 p-3.5 dark:border-neutral-800 dark:bg-neutral-900/50">
                      {provider.models.map((model) => (
                        <div
                          key={model.id}
                          className="flex items-center justify-between gap-3 rounded-lg border border-neutral-200 bg-white px-2.5 py-2 dark:border-neutral-800 dark:bg-neutral-800/60"
                        >
                          <span className="min-w-0 truncate text-xs font-medium text-neutral-800 dark:text-neutral-200">
                            {model.display_name}
                          </span>
                          <span className="shrink-0 font-mono text-[10px] text-neutral-500 dark:text-neutral-400">
                            {model.id}
                          </span>
                        </div>
                      ))}
                    </div>
                  </details>
                ) : null}
              </div>
            );
          })}
        </div>
      )}

      <Dialog.Root open={Boolean(connectTarget)} onOpenChange={(open) => !open && closeConnect()}>
        <Dialog.Content className="ui-dialog-compact" showClose={false}>
          <Dialog.Header>
            <div className="flex items-start justify-between gap-4">
              <div>
                <Dialog.Title>{t("Configure API key")}</Dialog.Title>
                <Dialog.Description>
                  {t("The key is stored locally and is never displayed again.")}
                </Dialog.Description>
              </div>
              <IconButton type="button" size="sm" label={t("Close")} onClick={closeConnect}>
                <X className="h-4 w-4" />
              </IconButton>
            </div>
          </Dialog.Header>
          <form onSubmit={(event) => void handleConnect(event)}>
            <div className="space-y-4">
              <Field
                label={connectTarget?.display_name}
                description={connectTarget?.id}
                error={connectError}
                required
                htmlFor="provider-api-key"
              >
                <Input
                  id="provider-api-key"
                  type="password"
                  value={apiKey}
                  onChange={(event) => setApiKey(event.target.value)}
                  placeholder={t("Enter API key")}
                  autoComplete="new-password"
                  autoFocus
                />
              </Field>
            </div>
            <Dialog.Footer>
              <Button type="button" variant="ghost" onClick={closeConnect} disabled={isConnecting}>
                {t("Cancel")}
              </Button>
              <Button type="submit" variant="primary" loading={isConnecting}>
                {t("Save key")}
              </Button>
            </Dialog.Footer>
          </form>
        </Dialog.Content>
      </Dialog.Root>

      <Dialog.Root open={addOpen} onOpenChange={(open) => !open && closeAdd()}>
        <Dialog.Content
          className="max-h-[calc(100dvh-32px)] w-[calc(100vw-32px)] max-w-3xl overflow-x-hidden overflow-y-auto p-0"
          showClose={false}
        >
          <Dialog.Header className="border-b border-neutral-200 px-5 py-4 dark:border-neutral-800 sm:px-6">
            <div className="flex items-start justify-between gap-4">
              <div>
                <Dialog.Title>{t("Add Provider")}</Dialog.Title>
                <Dialog.Description>
                  {t("Create a custom provider with at least one model.")}
                </Dialog.Description>
              </div>
              <IconButton type="button" size="sm" label={t("Close")} onClick={closeAdd}>
                <X className="h-4 w-4" />
              </IconButton>
            </div>
          </Dialog.Header>

          <form onSubmit={(event) => void handleCreate(event)}>
            <div className="space-y-4 px-5 py-4 sm:px-6">
              {formError ? (
                <div
                  className="rounded-lg bg-red-50 p-2.5 text-xs text-red-700 dark:bg-red-950/40 dark:text-red-300"
                  role="alert"
                >
                  {formError}
                </div>
              ) : null}

              <div className="space-y-3">
                <div className="grid gap-3 sm:grid-cols-2">
                  <Field label={t("Provider ID")} required htmlFor="provider-id">
                    <Input
                      id="provider-id"
                      value={draft.provider_id}
                      onChange={(event) =>
                        setDraft((current) => ({ ...current, provider_id: event.target.value }))
                      }
                      placeholder="my-provider"
                      className="font-mono"
                    />
                  </Field>
                  <Field label={t("Display name")} required htmlFor="provider-display-name">
                    <Input
                      id="provider-display-name"
                      value={draft.display_name}
                      onChange={(event) =>
                        setDraft((current) => ({ ...current, display_name: event.target.value }))
                      }
                      placeholder={t("My Provider")}
                    />
                  </Field>
                </div>

                <Field
                  label={t("Base URL")}
                  description={t(
                    "Use the provider base URL; tidev adds the protocol-specific path.",
                  )}
                  required
                  htmlFor="provider-base-url"
                >
                  <Input
                    id="provider-base-url"
                    value={draft.base_url}
                    onChange={(event) =>
                      setDraft((current) => ({ ...current, base_url: event.target.value }))
                    }
                    placeholder="https://api.example.com/v1"
                    className="font-mono"
                  />
                </Field>

                <div className="grid gap-3 sm:grid-cols-[minmax(0,1.15fr)_minmax(0,0.85fr)]">
                  <Select
                    label={t("API type")}
                    value={draft.api_type}
                    options={API_TYPE_OPTIONS}
                    triggerClassName="min-w-0 whitespace-nowrap"
                    onValueChange={(value) =>
                      setDraft((current) => ({ ...current, api_type: value }))
                    }
                  />
                  <Field label={t("API key")} required htmlFor="new-provider-api-key">
                    <Input
                      id="new-provider-api-key"
                      type="password"
                      value={draft.api_key}
                      onChange={(event) =>
                        setDraft((current) => ({ ...current, api_key: event.target.value }))
                      }
                      placeholder={t("Enter API key")}
                      autoComplete="new-password"
                    />
                  </Field>
                </div>
              </div>

              <div className="space-y-3 border-t border-neutral-200 pt-4 dark:border-neutral-800">
                <div className="flex items-start justify-between gap-3">
                  <div>
                    <div className="flex items-center gap-2">
                      <h3 className="text-xs font-semibold text-neutral-800 dark:text-neutral-200">
                        {t("Models")}
                      </h3>
                      <span className="rounded-full bg-neutral-100 px-2 py-0.5 text-[10px] font-medium text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300">
                        {t("{{count}} models", { count: draft.models.length })}
                      </span>
                    </div>
                    <p className="mt-1 text-[11px] text-neutral-500 dark:text-neutral-400">
                      {t("Define the models exposed by this provider.")}
                    </p>
                  </div>
                  <Button
                    type="button"
                    size="sm"
                    variant="secondary"
                    className="shrink-0"
                    leadingIcon={<Plus className="h-3.5 w-3.5" />}
                    onClick={addModel}
                  >
                    {t("Add model")}
                  </Button>
                </div>

                <div className="grid gap-3 md:grid-cols-[168px_minmax(0,1fr)]">
                  <div className="space-y-2">
                    {draft.models.map((model, index) => (
                      <button
                        key={index}
                        type="button"
                        className={`flex w-full items-center justify-between gap-2 rounded-lg border px-3 py-2.5 text-left transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] ${
                          index === activeModelIndex
                            ? "border-indigo-300 bg-indigo-50 text-indigo-950 dark:border-indigo-700 dark:bg-indigo-950/40 dark:text-indigo-100"
                            : "border-neutral-200 bg-white text-neutral-700 hover:border-neutral-300 hover:bg-neutral-50 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-300 dark:hover:border-neutral-700 dark:hover:bg-neutral-800"
                        }`}
                        onClick={() => setActiveModelIndex(index)}
                        aria-pressed={index === activeModelIndex}
                      >
                        <span className="min-w-0">
                          <span className="block truncate text-xs font-semibold">
                            {model.display_name.trim() ||
                              t("Model {{number}}", { number: index + 1 })}
                          </span>
                          <span className="mt-0.5 block truncate font-mono text-[10px] opacity-70">
                            {model.model_id || t("Model ID")}
                          </span>
                        </span>
                        <span className="shrink-0 text-[10px] font-medium opacity-60">
                          {index + 1}
                        </span>
                      </button>
                    ))}
                  </div>

                  {activeModel ? (
                    <div className="space-y-3 rounded-xl border border-neutral-200 bg-neutral-50/60 p-3 dark:border-neutral-800 dark:bg-neutral-900/60">
                      <div className="flex items-center justify-between gap-3 border-b border-neutral-200 pb-2.5 dark:border-neutral-800">
                        <span className="text-xs font-semibold text-neutral-800 dark:text-neutral-200">
                          {t("Model {{number}}", { number: activeModelIndex + 1 })}
                        </span>
                        {draft.models.length > 1 ? (
                          <IconButton
                            type="button"
                            size="sm"
                            variant="danger"
                            label={t("Remove model")}
                            onClick={() => removeModel(activeModelIndex)}
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </IconButton>
                        ) : null}
                      </div>
                      <div className="grid gap-2.5 sm:grid-cols-2">
                        <Field label={t("Model ID")} required htmlFor="active-model-id">
                          <Input
                            id="active-model-id"
                            value={activeModel.model_id}
                            onChange={(event) =>
                              updateModel(activeModelIndex, { model_id: event.target.value })
                            }
                            placeholder="gpt-4o"
                            className="font-mono"
                          />
                        </Field>
                        <Field label={t("Display name")} required htmlFor="active-model-name">
                          <Input
                            id="active-model-name"
                            value={activeModel.display_name}
                            onChange={(event) =>
                              updateModel(activeModelIndex, { display_name: event.target.value })
                            }
                            placeholder="GPT-4o"
                          />
                        </Field>
                        <Field label={t("Request model ID")} htmlFor="active-request-model-id">
                          <Input
                            id="active-request-model-id"
                            value={activeModel.request_model_id}
                            onChange={(event) =>
                              updateModel(activeModelIndex, {
                                request_model_id: event.target.value,
                              })
                            }
                            placeholder={t("Same as Model ID")}
                            className="font-mono"
                          />
                        </Field>
                        <Field label={t("Temperature")} required htmlFor="active-temperature">
                          <Input
                            id="active-temperature"
                            type="number"
                            step="any"
                            value={activeModel.temperature}
                            onChange={(event) =>
                              updateModel(activeModelIndex, { temperature: event.target.value })
                            }
                          />
                        </Field>
                        <Field label={t("Context window")} required htmlFor="active-context-window">
                          <Input
                            id="active-context-window"
                            type="number"
                            min="1"
                            step="1"
                            value={activeModel.context_window}
                            onChange={(event) =>
                              updateModel(activeModelIndex, { context_window: event.target.value })
                            }
                          />
                        </Field>
                        <Field
                          label={t("Max output tokens")}
                          required
                          htmlFor="active-max-output-tokens"
                        >
                          <Input
                            id="active-max-output-tokens"
                            type="number"
                            min="1"
                            step="1"
                            value={activeModel.max_output_tokens}
                            onChange={(event) =>
                              updateModel(activeModelIndex, {
                                max_output_tokens: event.target.value,
                              })
                            }
                          />
                        </Field>
                      </div>
                      <label className="flex items-center gap-2 border-t border-neutral-200 pt-2.5 text-xs text-neutral-600 dark:border-neutral-800 dark:text-neutral-400">
                        <input
                          type="checkbox"
                          checked={activeModel.supports_images}
                          onChange={(event) =>
                            updateModel(activeModelIndex, { supports_images: event.target.checked })
                          }
                        />
                        {t("Supports images")}
                      </label>
                    </div>
                  ) : null}
                </div>
              </div>
            </div>
            <Dialog.Footer className="mt-0 border-t border-neutral-200 px-5 py-3 sm:px-6 dark:border-neutral-800">
              <Button type="button" variant="secondary" onClick={closeAdd} disabled={isCreating}>
                {t("Cancel")}
              </Button>
              <Button type="submit" variant="primary" loading={isCreating}>
                {t("Create provider")}
              </Button>
            </Dialog.Footer>
          </form>
        </Dialog.Content>
      </Dialog.Root>

      {providerToDelete ? (
        <ConfirmDialog
          danger
          title={t("Delete provider")}
          message={t(
            'Delete provider "{{name}}"? Its API key and model configuration will be removed.',
            {
              name: providerToDelete.display_name,
            },
          )}
          confirmText={t("Delete")}
          cancelText={t("Cancel")}
          isLoading={isDeleting}
          onConfirm={() => void handleDelete()}
          onCancel={() => {
            if (!isDeleting) setProviderToDelete(null);
          }}
        />
      ) : null}
    </section>
  );
}
