import { createSignal } from "solid-js";
import { rawText, renderEnglish, textKey, type UiText } from "./i18n";

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

/** A toast's text: a catalog message or `rawText`; a bare string is a legacy,
 *  not-yet-translated site (tine#107; scripts/i18n-literals.mjs counts them). */
export type ToastText = UiText | string;
type ToastAction = { label: ToastText; run: () => void };

export interface Toast {
  id: number;
  /** What the toast shows, resolved in the active locale where it is rendered. */
  text: UiText;
  /** The English text: what Copy, the session error list and the debug log get,
   *  so a report reads the same in every UI language (G5). */
  message: string;
  /** Dedupe identity: the message id and English text, or the raw text. */
  key: string;
  kind: "info" | "success" | "warn" | "error";
  // Stays until the user closes it (✕); no auto-dismiss. Every "error" toast is
  // sticky: an error means something actually went wrong and is worth reporting
  // (Martin, 2026-09-29), so it must be readable and copyable, not a 3 s flash.
  sticky?: boolean;
  count?: number; // identical sticky errors shown so far (rendered "×N" above 1)
  // Optional action button (e.g. "Download"). Runs, then dismisses the toast.
  action?: ToastAction;
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
export function toUiText(text: ToastText): UiText {
  return typeof text === "string" ? rawText(text) : text;
}
export function pushToast(
  content: ToastText,
  kind: Toast["kind"] = "info",
  opts: { sticky?: boolean; action?: ToastAction; onDismiss?: () => void } = {}
): number {
  const text = toUiText(content);
  const message = renderEnglish(text);
  const key = textKey(text);
  // A repeated identical error (a retrying write) is one toast with a count,
  // not a growing wall of red; every occurrence is still recorded.
  const repeated = kind === "error" && !opts.action && !opts.onDismiss ? toasts().find((toast) =>
    toast.kind === "error" && toast.key === key && !toast.action && !toast.onDismiss) : undefined;
  if (repeated) {
    setToasts(toasts().map((toast) => toast.id === repeated.id ? { ...toast, count: (toast.count ?? 1) + 1 } : toast));
    errorToastRecorder?.(message);
    return repeated.id;
  }
  const id = ++toastSeq;
  const sticky = kind === "error" || opts.sticky;
  setToasts([...toasts(), { id, text, message, key, kind, sticky, action: opts.action, onDismiss: opts.onDismiss }]);
  if (kind === "error") errorToastRecorder?.(message);
  if (!sticky) setTimeout(() => dismissToast(id), 3200);
  return id;
}
/** Return the existing ID for an identical visible status, or create one.
 * Cost O(visible toasts); no failure beyond pushToast's signal update. */
export function pushToastUnique(
  content: ToastText,
  kind: Toast["kind"],
  opts: { sticky?: boolean; action?: ToastAction; onDismiss?: () => void } = {}
): number {
  const key = textKey(toUiText(content));
  const existing = toasts().find((toast) => toast.key === key && toast.kind === kind);
  return existing?.id ?? pushToast(content, kind, opts);
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
