export type EditableEmojiPlatform = "windows" | "apple" | "android" | "safe-monochrome";

export function editableEmojiPlatform(userAgent: string): EditableEmojiPlatform {
  if (/Android/i.test(userAgent)) return "android";
  if (/Windows/i.test(userAgent)) return "windows";
  if (/(Macintosh|Mac OS|iPhone|iPad|iPod)/i.test(userAgent)) return "apple";
  return "safe-monochrome";
}

/** Select the emoji policy before first paint. Windows and Apple display
 * surfaces share the editable font; other platforms retain Twemoji display. */
export function installEditableEmojiPlatform(userAgent = navigator.userAgent): EditableEmojiPlatform {
  const platform = editableEmojiPlatform(userAgent);
  document.documentElement.dataset.editableEmoji = platform;
  return platform;
}

/** Read the installed policy, rather than independently detecting the platform.
 * An absent/unknown policy must retain the crash-safe SVG display path. */
export function usesNativeEmojiDisplay(): boolean {
  const platform = document.documentElement.dataset.editableEmoji;
  return platform === "windows" || platform === "apple";
}
