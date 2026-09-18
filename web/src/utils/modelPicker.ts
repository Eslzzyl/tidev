import type { Model } from "../types/api";

export interface ModelProviderGroup {
  id: string;
  name: string;
  connected: boolean;
  models: Model[];
}

export function groupModelsByProvider(models: Model[]): ModelProviderGroup[] {
  const groups = new Map<string, ModelProviderGroup>();

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
}

export function filterModelProviderGroups(
  providers: ModelProviderGroup[],
  query: string,
): ModelProviderGroup[] {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  if (!normalizedQuery) return providers;

  return providers.flatMap((provider) => {
    const providerMatches = [provider.id, provider.name].some((value) =>
      value.toLocaleLowerCase().includes(normalizedQuery),
    );
    const models = providerMatches
      ? provider.models
      : provider.models.filter((model) =>
          [model.model_id, model.model_display_name].some((value) =>
            value.toLocaleLowerCase().includes(normalizedQuery),
          ),
        );

    return models.length ? [{ ...provider, models }] : [];
  });
}
