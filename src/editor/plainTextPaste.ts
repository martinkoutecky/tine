import { browserPlatform } from "../browserPlatform";
import { readClipboardText } from "../clipboard";
import { graphOwner, readOwned } from "../owned";
import { msg } from "../i18n";
import { pushToast } from "../toasts";
import { shortcutKey } from "../shortcutKey";

interface PasteTarget {
  current(): boolean;
  insert(text: string): void;
}

/** macOS has no guaranteed native paste for Cmd+Shift+V: read plain text
 * explicitly, as OG's editor-on-paste-raw! does. Other platforms retain their
 * synchronous native clipboard event. */
export function plainTextPaste() {
  let raw = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let requestId = 0;
  const clearRaw = () => {
    raw = false;
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };
  const request = (capture: () => PasteTarget | null) => {
    clearRaw();
    const id = ++requestId;
    const target = capture();
    if (!target?.current()) return;
    const owner = graphOwner(() => id === requestId && target.current());
    void readOwned(owner, readClipboardText()).then(result => {
      if (result.kind === "current" && result.value) target.insert(result.value);
    }).catch(() => {
      if (owner()) pushToast(msg("editor-block-couldn-t-read-the-clipboard"), "error");
    });
  };
  const keydown = (e: KeyboardEvent, capture: () => PasteTarget | null): boolean => {
    const mac = browserPlatform("", navigator.platform).macKeyboard;
    const match = !e.isComposing && e.keyCode !== 229 && e.shiftKey && shortcutKey(e) === "v" && (e.ctrlKey || e.metaKey);
    clearRaw();
    if (!match || mac && e.altKey) return false;
    if (mac && e.metaKey) {
      e.preventDefault();
      request(capture);
    } else {
      raw = true;
      timer = setTimeout(clearRaw, 1_000);
    }
    return true;
  };
  return {
    keydown, clearRaw,
    consume: () => { const wasRaw = raw; clearRaw(); return wasRaw; },
    dispose: () => { clearRaw(); ++requestId; },
  };
}
