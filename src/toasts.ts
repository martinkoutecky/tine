import { createSignal } from "solid-js";

export interface Toast {
  id: number;
  message: string;
  kind: "info" | "success" | "warn" | "error";
  // Stays until the user closes it (✕); no auto-dismiss. Every "error" toast is
  // sticky: an error means something actually went wrong and is worth reporting
  // (Martin, 2026-09-29), so it must be readable and copyable, not a 3 s flash.
  sticky?: boolean;
  // Optional action button (e.g. "Download"). Runs, then dismisses the toast.
  action?: { label: string; run: () => void };
  onDismiss?: () => void;
}
let toastSeq = 0;
export const [toasts, setToasts] = createSignal<Toast[]>([]);
let errorToastRecorder: ((message: string) => void) | null = null;
/** Record every error toast shown (debug.ts wires the flight recorder and the
 *  opt-in debug log here), so a report can be recovered after it is closed. */
export function recordErrorToastsWith(record: ((message: string) => void) | null): void {
  errorToastRecorder = record;
}
export function pushToast(
  message: string,
  kind: Toast["kind"] = "info",
  opts: { sticky?: boolean; action?: { label: string; run: () => void }; onDismiss?: () => void } = {}
): number {
  const id = ++toastSeq;
  const sticky = kind === "error" || opts.sticky;
  setToasts([...toasts(), { id, message, kind, sticky, action: opts.action, onDismiss: opts.onDismiss }]);
  if (kind === "error") errorToastRecorder?.(message);
  if (!sticky) setTimeout(() => dismissToast(id), 3200);
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
