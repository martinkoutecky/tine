import { msg } from "./i18n";
/** Device-local code wrapping, shared by every rendered code card and editor.
 * Persistence is O(1); viewport capture scans mounted reading blocks.
 * Startup and changes use the existing app-settings
 * door. Failures report a toast; a delayed startup read cannot undo a user change.
 * Language/body structure remains the parser's answer, never this preference.
 */
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { pushToast } from "./toasts";
import { advanceRevision, currentRevision, readOwned, revisionOwner, writeOwned } from "./owned";
import { captureReadingViewport, readingScrollerSelector } from "./components/readingViewportAnchor";
import { captureEditorScrollAnchor } from "./editor/scrollAnchor";
import { isElementTag, queryAllWindowsAll, requestFrame } from "./windowRealm";

const key = "code_line_wrapping";
const slot = {};
const [codeWrapping, setWrapping] = createSignal(false);
export { codeWrapping };

export async function initCodeDisplay(): Promise<void> {
  try {
    const loaded = await readOwned(revisionOwner(slot, currentRevision(slot)), backend().getAppString(key, "0"));
    if (loaded.kind === "current") setWrapping(loaded.value === "1");
  } catch (error) {
    pushToast(msg("code-display-could-not-load-code-wrapping", { error: String(error) }), "error");
  }
}

export function changeCodeWrapping(value: boolean): void {
  // The global preference reflows every mounted surface, including inactive
  // splits and workspace windows. A focused editor owns its scroller; reading
  // anchors are only captured in the remaining surfaces.
  const anchors = value === codeWrapping() ? [] : queryAllWindowsAll<HTMLElement>(readingScrollerSelector).map(scroller => {
    const focused = scroller.ownerDocument.activeElement;
    if (isElementTag(focused, "textarea") && scroller.contains(focused)) {
      const anchor = captureEditorScrollAnchor(focused, scroller);
      // Wait for Solid's current batch to schedule Block's coalesced autosize,
      // then restore in that same frame after the editor has measured its body.
      return { restore: () => queueMicrotask(() => requestFrame(scroller, () => anchor?.restore())) };
    }
    return captureReadingViewport(scroller);
  });
  setWrapping(value);
  for (const anchor of anchors) anchor?.restore();
  const owner = revisionOwner(slot, advanceRevision(slot));
  void writeOwned(owner, backend().setAppString(key, value ? "1" : "0"))
    .catch(error => pushToast(msg("code-display-could-not-remember-code-wrapping", { error: String(error) }), "error"));
}
