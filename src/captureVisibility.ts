/**
 * Reconcile a capture frontend that finished installing its event listeners
 * after Rust had already shown the native window. This is deliberately kept
 * separate from the component so the missed-event lifecycle is deterministic
 * under test.
 */
export async function resettleIfVisible(
  window: { isVisible(): Promise<boolean> },
  resettle: () => void,
): Promise<void> {
  if (await window.isVisible()) resettle();
}
