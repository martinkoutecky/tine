import { isTauri } from "./backend";
import { platformKind, type PlatformKind } from "./nativeChrome";

/** Which layer excludes the system bars: the native Android content root, or
 *  the page through CSS env(safe-area-inset-*). */
export type SystemInsetOwner = "native-viewport" | "css-viewport";

/**
 * The sole system-inset owner for a surface (GH #205). Only a native Tine host
 * on Android has MainActivity padding the WebView inside the system-bar and
 * cutout insets; a browser (including Android Chrome viewing a published
 * export) and iOS keep CSS ownership. Pure; O(1).
 */
export function systemInsetOwner(nativeHost: boolean, platform: PlatformKind): SystemInsetOwner {
  return nativeHost && platform === "android" ? "native-viewport" : "css-viewport";
}

/**
 * Publish the owner as `data-system-insets` on the root element before any
 * application UI renders, so app.css zeroes the `--system-inset-*` tokens
 * where the native viewport is already inset and no band is reserved twice.
 * Returns the owner it published. Call once from main.tsx before applyTheme().
 */
export function installSystemInsetOwner(
  root: HTMLElement = document.documentElement,
  nativeHost: boolean = isTauri(),
  platform: PlatformKind = platformKind,
): SystemInsetOwner {
  const owner = systemInsetOwner(nativeHost, platform);
  root.dataset.systemInsets = owner;
  return owner;
}
