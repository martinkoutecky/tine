import { createSignal } from "solid-js";

export interface Toast {
  id: number;
  message: string;
  kind: "info" | "success" | "warn" | "error";
  sticky?: boolean; // stays until the user closes it (✕); no auto-dismiss
  // Optional action button (e.g. "Download"). Runs, then dismisses the toast.
  action?: { label: string; run: () => void };
  onDismiss?: () => void;
}
let toastSeq = 0;
export const [toasts, setToasts] = createSignal<Toast[]>([]);
export function pushToast(
  message: string,
  kind: Toast["kind"] = "info",
  opts: { sticky?: boolean; action?: { label: string; run: () => void }; onDismiss?: () => void } = {}
): number {
  const id = ++toastSeq;
  setToasts([...toasts(), { id, message, kind, sticky: opts.sticky, action: opts.action, onDismiss: opts.onDismiss }]);
  if (!opts.sticky) setTimeout(() => dismissToast(id), 3200);
  return id;
}
/** Return the existing ID for an identical visible status, or create one.
 * Cost O(visible toasts); no failure beyond pushToast's signal update. */
export function pushToastUnique(
  message: string,
  kind: Toast["kind"],
  opts: { sticky?: boolean; action?: { label: string; run: () => void }; onDismiss?: () => void } = {}
): number {
  const existing = toasts().find((toast) => {
    const text = toast.message;
    return text === message && toast.kind === kind;
  });
  return existing?.id ?? pushToast(message, kind, opts);
}
export function dismissToast(id: number) {
  const toast = toasts().find((t) => t.id === id);
  toast?.onDismiss?.();
  setToasts(toasts().filter((t) => t.id !== id));
}

// Full-screen image lightbox (click an inline image to zoom).
