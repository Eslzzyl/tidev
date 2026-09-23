import type { ProviderInfo } from "../types/api";

export function hasConnectedProvider(
  providers: readonly Pick<ProviderInfo, "connected">[],
): boolean {
  return providers.some((provider) => provider.connected);
}
