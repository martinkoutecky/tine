/** Call only after IME and popup ownership. Keep capture intent independent of
 * the editor's projected buffer and ordinary outline key handling. */
export function captureKeyIntent(event: KeyboardEvent, command: string | null, capture: { enterFiles(): boolean } | null): "submit" | "cancel" | null {
  if (!capture) return null;
  if (command === "editor/quick-capture-file") return "submit";
  if (event.key === "Enter" && !event.shiftKey && !event.ctrlKey && !event.metaKey && capture.enterFiles()) return "submit";
  if (event.key === "Escape") return "cancel";
  return null;
}
