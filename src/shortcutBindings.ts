/** OG :shortcuts values: one binding or a vector of alternatives. The legacy
 * local "false" sentinel still disables a command. */
export type ShortcutBinding = string | string[];

export function bindingAlternatives(value: ShortcutBinding): string[] {
  return (Array.isArray(value) ? value : [value])
    .filter((binding) => typeof binding === "string" && !!binding.trim() && binding !== "false")
    .map((binding) => binding.trim());
}

/** Keep existing single-key settings byte-shape compatible. */
export function shortcutValue(bindings: string[]): ShortcutBinding {
  return bindings.length === 0 ? "false" : bindings.length === 1 ? bindings[0] : bindings;
}
