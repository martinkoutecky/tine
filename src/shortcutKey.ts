/** Shortcut identity only: never use this to transform editor input.
 * OG's Closure handler matches Latin key names plus legacy keyCodes. Use code
 * for a non-Latin character, where key cannot name the Latin binding, while
 * keeping layout-produced Latin letters and ASCII punctuation (AZERTY/Dvorak).
 * Named keys, dead keys and events without a physical code keep their meaning.
 */
export function shortcutKey(e: Pick<KeyboardEvent, "key" | "code">): string {
  const key = e.key.toLowerCase();
  if (/[^\u0000-\u007f]/u.test(key) && !/\p{Script=Latin}/u.test(key)) {
    const physical = /^(?:Key([A-Z])|Digit([0-9]))$/.exec(e.code);
    if (physical) return (physical[1] ?? physical[2]).toLowerCase();
  }
  return key;
}
