import { createSignal } from "solid-js";

type ReferenceChangeCounter = (ids: readonly string[]) => number | null | undefined | Promise<number | null | undefined>;
let referenceChangeCounter: ReferenceChangeCounter = () => undefined;
/** The count service installs its reader here: document edits can request a
 * notice without importing the graph count service back into the document. */
export function installReferenceChangeCounter(read: ReferenceChangeCounter): void {
  referenceChangeCounter = read;
}
export function captureReferenceChangeCount(ids: readonly string[]): ReturnType<ReferenceChangeCounter> {
  return ids.length ? referenceChangeCounter(ids) : 0;
}

export interface Toast {
  id: number;
  message: string;
  kind: "info" | "success" | "warn" | "error";
  // Stays until the user closes it (✕); no auto-dismiss. Every "error" toast is
  // sticky: an error means something actually went wrong and is worth reporting
  // (Martin, 2026-09-29), so it must be readable and copyable, not a 3 s flash.
  sticky?: boolean;
  count?: number; // identical sticky errors shown so far (rendered "×N" above 1)
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
  // A repeated identical error (a retrying write) is one toast with a count,
  // not a growing wall of red; every occurrence is still recorded.
  const repeated = kind === "error" && !opts.action && !opts.onDismiss ? toasts().find((toast) => {
    const text = toast.message;
    return toast.kind === "error" && text === message && !toast.action && !toast.onDismiss;
  }) : undefined;
  if (repeated) {
    setToasts(toasts().map((toast) => toast.id === repeated.id ? { ...toast, count: (toast.count ?? 1) + 1 } : toast));
    errorToastRecorder?.(message);
    return repeated.id;
  }
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

/** One notice vocabulary for deletion and both keyboard merge directions. */
export function pushReferenceChangeNotice(count: number | null, transferred: boolean, undo: () => void): number | null {
  if (count === 0) return null;
  const message = count === null
    ? transferred ? "References may now point to this block" : "References may now be broken"
    : transferred
      ? `${count} ${count === 1 ? "reference now points" : "references now point"} to this block`
      : `${count} ${count === 1 ? "reference is" : "references are"} now broken`;
  let id = 0;
  id = pushToast(message, "warn", { sticky: true, action: { label: "Undo", run: () => { undo(); dismissToast(id); } } });
  return id;
}

// Full-screen image lightbox (click an inline image to zoom).
