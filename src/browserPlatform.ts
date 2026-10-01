/** Browser hints for contexts without native platform information. O(input
 * length), synchronous and pure; never asks IPC during an input event. Native
 * injected platform values remain authoritative for window/mobile behavior.
 * Consumers select named policies: emoji font support, keyboard convention and
 * unsigned desktop update installation are different questions.
 */
export function browserPlatform(userAgent: string, navigatorPlatform = "") {
  return {
    android: /Android/i.test(userAgent),
    ios: /iPhone|iPad|iPod/i.test(userAgent),
    windows: /Windows/i.test(userAgent),
    appleEmoji: /(Macintosh|Mac OS|iPhone|iPad|iPod)/i.test(userAgent),
    macDesktop: /Mac/i.test(navigatorPlatform) || /Mac OS X|Macintosh/i.test(userAgent),
    macKeyboard: /Mac/.test(navigatorPlatform),
    manualDesktopUpdate: /\bMac/i.test(userAgent),
  };
}
