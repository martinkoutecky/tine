/**
 * Reconcile a capture frontend that finished installing its event listeners
 * after Rust had already shown the native window. This is deliberately kept
 * separate from the component so the missed-event lifecycle is deterministic
 * under test.
 */
export async function resettleIfVisible(
  window: { isVisible(): Promise<boolean> },
  resettle: () => void | Promise<void>,
): Promise<void> {
  if (await window.isVisible()) await resettle();
}

/**
 * A newly mapped auxiliary window can report an unfocused transition before
 * the window manager has honored its first activation request. Treating that
 * transition as an ordinary user blur hides Quick Capture before the bounded
 * native focus retries can run. Arm blur-to-dismiss only after this show has
 * actually owned focus; explicit hides disarm the next stale transition too.
 */
export function createCaptureBlurGate(
  now: () => number = Date.now,
  stableFocusMs = 200,
): {
  focusChanged(focused: boolean): boolean;
  disarm(): void;
} {
  let focusedAt: number | null = null;
  return {
    focusChanged(focused) {
      if (focused) {
        focusedAt = now();
        return false;
      }
      if (focusedAt === null) return false;
      const heldFocusLongEnough = now() - focusedAt >= stableFocusMs;
      focusedAt = null;
      return heldFocusLongEnough;
    },
    disarm() {
      focusedAt = null;
    },
  };
}
