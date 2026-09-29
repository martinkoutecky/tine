import { initThemeGallery } from "../themeGallery";
import { initThemePackages } from "../themes/manager";
import { pluginManager } from "./manager";
import {
  loadVerifiedCachedRegistry,
  refreshCommunityRegistry,
  seedCachedCommunityRegistry,
} from "./registry";

export interface CommunityExtensionStartup {
  initialRevocations: ReadonlySet<string>;
  pluginInitialization: Promise<void>;
  liveRefresh: Promise<void>;
}

/** Observe the detached rejection while preserving it on the returned promise;
 * main.tsx reports that failure to the user. No extra work is started. */
function observePluginInitializationFailure(_error: unknown): void {}

/** Seed verified extensions and return the initialization and refresh tasks.
 * Cost O(installed extensions) plus one registry request. Initialization may
 * reject on the returned promise; main.tsx reports that failure to the user. */
export async function startCommunityExtensions(
  options: { cacheTimeoutMs?: number; networkTimeoutMs?: number } = {}
): Promise<CommunityExtensionStartup> {
  const cached = await loadVerifiedCachedRegistry(options.cacheTimeoutMs);
  const initialRevocations = seedCachedCommunityRegistry(cached);
  const activationHeld = cached.kind === "unsafe";

  // Calling an async function runs through its first await synchronously.
  // initialize() and initThemePackages() therefore seed the same verified set
  // before either path can load or activate persisted state. The live refresh
  // starts immediately after initialize() yields and is not chained to whether
  // any persisted plugin later succeeds or fails.
  const pluginInitialization = pluginManager.initialize(initialRevocations, activationHeld);
  void pluginInitialization.then(undefined, observePluginInitializationFailure);
  const liveRefresh = refreshCommunityRegistry({ timeoutMs: options.networkTimeoutMs });
  await initThemePackages(initialRevocations);
  await initThemeGallery();

  return { initialRevocations, pluginInitialization, liveRefresh };
}
