import type { Accessor } from "solid-js";
import { backend } from "../backend";
import { reportUiFailure } from "../uiFailure";

/** Open one link, reporting the existing fixed UI failure on rejection. */
export function openExternal(url: string): void {
  void backend().openExternal(url).catch((error) => reportUiFailure("external-link", error));
}

/** Constant-time Euclidean month stepping; m is zero-based, including negative years. */
export function stepMonth(view: { y: number; m: number }, delta: number): { y: number; m: number } {
  const total = view.y * 12 + view.m + delta;
  return { y: Math.floor(total / 12), m: ((total % 12) + 12) % 12 };
}

/** Merge, publish and persist exactly one options object, with no effects or extra copies. */
export function optionsUpdater<T extends object>(read: Accessor<T>, set: (next: T) => unknown, save: (next: T) => void) {
  return (patch: Partial<T>): void => {
    const next = { ...read(), ...patch };
    set(next);
    save(next);
  };
}
