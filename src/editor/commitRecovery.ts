import { onCleanup } from "solid-js";
import { registerEditExitGuard } from "../editorController";
import { msg } from "../i18n";
import { dismissToast, pushToast } from "../toasts";

/** A refused join retains the mounted buffer and uses existing error/Copy
 * recovery if its whole surface closes. Never save a projection that could
 * drop hidden history. State ends with this editor; no new persisted draft. */
export function editorCommitRecovery(
  editor: () => HTMLTextAreaElement | undefined,
  current: () => boolean,
) {
  let release: (() => void) | undefined;
  let toast: number | undefined;
  let detail = "";
  const applied = () => { release?.(); release = undefined; };
  const report = (text: string) => {
    if (toast !== undefined) dismissToast(toast);
    // Separate editors own separate recovery copies, even for identical errors.
    // An owned dismissal also opts out of ordinary error-message deduplication.
    toast = pushToast(msg("editor-block-edit-refused", { detail }), "error", {
      copyText: text, onDismiss: () => { toast = undefined; },
    });
  };
  onCleanup(() => {
    // Routing/reload can remove the whole surface despite an edit-mode hold.
    // Retain the final buffer even if the earlier error was dismissed.
    const ref = editor();
    if (release && ref) report(ref.value);
    applied();
  });
  return {
    applied,
    refused(error: unknown, text: string): never {
      release ??= registerEditExitGuard(() => {
        const ref = editor();
        if (!ref?.isConnected || !current()) return true;
        ref.focus();
        return false;
      });
      detail = String(error);
      report(text);
      // Commit-first keyboard/blur/structural callers must stop here too.
      throw error;
    },
  };
}
