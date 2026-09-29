import { initThemeGallery } from "../themeGallery";
import { initThemePackages } from "../themes/manager";
import { platformKind } from "../platform";
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
 * On iOS the Wasm plugin host is never initialized (ADR 0052); themes still
 * load. Cost O(installed extensions) plus one registry request. Initialization may
 * reject on the returned promise; main.tsx reports that failure to the user. */
export async function startCommunityExtensions(
  options: {
    cacheTimeoutMs?: number;
    networkTimeoutMs?: number;
    /** Test-only override; production always uses the native platform result. */
    platform?: "desktop" | "android" | "ios";
  } = {}
): Promise<CommunityExtensionStartup> {
  const platform = options.platform ?? await platformKind();
  const cached = await loadVerifiedCachedRegistry(options.cacheTimeoutMs);
  const initialRevocations = seedCachedCommunityRegistry(cached);
  const activationHeld = cached.kind === "unsafe";

  // Calling an async function runs through its first await synchronously.
  // initialize() and initThemePackages() therefore seed the same verified set
  // before either path can load or activate persisted state. The live refresh
  // starts immediately after initialize() yields and is not chained to whether
  // any persisted plugin later succeeds or fails.
  // ADR 0052 / the accepted iOS v1 scope: downloadable Wasm plugins remain off
  // until Tine has the additional Apple 4.7 catalogue, reporting, age-rating,
  // and review surface. Themes are inert token manifests and remain available.
  // Do not replace this with a manifest-level `platforms` filter: even a local
  // iOS-marked package must not instantiate in the first App Store build.
  const pluginInitialization = platform === "ios"
    ? Promise.resolve()
    : pluginManager.initialize(initialRevocations, activationHeld);
  void pluginInitialization.then(undefined, observePluginInitializationFailure);
  const liveRefresh = refreshCommunityRegistry({ timeoutMs: options.networkTimeoutMs });
  await initThemePackages(initialRevocations);
  await initThemeGallery();

  return { initialRevocations, pluginInitialization, liveRefresh };
}
