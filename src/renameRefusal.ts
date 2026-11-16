import { msg, type Msg } from "./i18n";
import { pushToast, pushToastUnique } from "./toasts";
import type { RenameRefusal } from "./types";

/** Catalog-only notice boundary shared by refusal and recovery outcomes. */
export function showRenameNotice(message: Msg, kind: "info" | "error", options: Parameters<typeof pushToastUnique>[2] = {}): void {
  pushToastUnique(message, kind, options);
}

/** Localize the backend's refusal data; no interpretation of error prose. */
export function renameRefusalMessage(reason: RenameRefusal): Msg {
  switch (reason.kind) {
    case "taken": return msg("rename-refused-taken", { file: reason.into });
    case "checkRefused": return reason.guarantee === "P3"
      ? msg("rename-refused-first-save", { file: reason.file })
      : msg("rename-refused-check-failed", { file: reason.file });
    case "headerChildren": return msg("rename-refused-header-children", { file: reason.file });
    case "hiddenOccurrence": return msg("rename-refused-hidden-occurrence", { file: reason.file });
    case "survivorIdentity": return msg("rename-refused-survivor-identity", { file: reason.file });
    case "ownChild": return msg("rename-refused-own-child", { old: reason.old, new: reason.new });
    case "fileTaken": return msg("rename-refused-file-taken", { file: reason.file, page: reason.page });
    case "notAPage": return msg("rename-refused-not-a-page", { name: reason.name });
    case "day": return msg("rename-refused-day", { name: reason.name });
    case "aliasHeld": return msg("rename-refused-alias-held", { alias: reason.alias, page: reason.page });
    case "conflictMarkers": return msg("rename-refused-conflict-markers", { file: reason.file });
    case "capture": return msg("rename-refused-capture", { name: reason.name, image: reason.image, page: reason.page });
    case "contested": return msg("rename-refused-contested", { name: reason.name, files: reason.files.join(", ") });
  }
}

/** Show a typed transaction refusal on the existing sticky error surface.
 * The optional recovery performs one explicitly requested save then retry;
 * another refusal offers another click. Cost O(visible toasts), no I/O until
 * clicked. Returns false for ordinary I/O errors so their owner handles them. */
export function showRewriteRefusal(error: unknown, retry?: (file: string) => Promise<void>): boolean {
  if (!error || typeof error !== "object" || !("refused" in error)) return false;
  const reason = error.refused as RenameRefusal;
  const action = reason.kind === "checkRefused" && reason.guarantee === "P3" && retry
    ? { label: msg("rename-save-and-retry"), run: () => {
      void retry(reason.file).catch((failure: unknown) => {
        if (!showRewriteRefusal(failure, retry)) pushToast(msg("rename-recovery-failed", { error: String(failure) }), "error");
      });
    } } : undefined;
  showRenameNotice(renameRefusalMessage(reason), "error", { action });
  return true;
}
