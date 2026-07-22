import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import {
  clearTransientLayersForTest,
  dismissTopTransient,
  registerTransientLayer,
} from "../transientLayers";
import { CalendarJump } from "./CalendarJump";

afterEach(() => {
  clearTransientLayersForTest();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

function mountCalendar(count = 1) {
  vi.spyOn(backend(), "journalContentDays").mockResolvedValue([]);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => (
    <>{Array.from({ length: count }, () => <CalendarJump />)}</>
  ), root);
  return { root, dispose };
}

function lowerSentinel(id: string) {
  let dismissals = 0;
  const unregister = registerTransientLayer({
    id,
    dismiss: () => { dismissals += 1; return true; },
  });
  return { unregister, dismissals: () => dismissals };
}

describe("CalendarJump transient ownership", () => {
  it("exposes the same date-picker opener for a compact toolbar parent", async () => {
    let openFromOverflow: (() => void) | undefined;
    const root = document.createElement("div");
    document.body.append(root);
    vi.spyOn(backend(), "journalContentDays").mockResolvedValue([]);
    const dispose = render(() => <CalendarJump onOpenReady={(open) => { openFromOverflow = open; }} />, root);
    try {
      await vi.waitFor(() => expect(openFromOverflow).toBeTypeOf("function"));
      openFromOverflow!();
      await vi.waitFor(() => expect(root.querySelector(".calendar-jump-pop")).not.toBeNull());
    } finally {
      dispose();
    }
  });

  it.each(["escape", "back"] as const)("owns one %s rung and restores its real trigger", async (reason) => {
    const lower = lowerSentinel(`calendar-lower-${reason}`);
    const { root, dispose } = mountCalendar();
    try {
      const trigger = root.querySelector<HTMLButtonElement>('button[title="Go to date"]')!;
      trigger.click();
      await vi.waitFor(() => expect(root.querySelector(".calendar-jump-pop")).not.toBeNull());

      expect(dismissTopTransient(reason)).toBe(true);
      await vi.waitFor(() => expect(root.querySelector(".calendar-jump-pop")).toBeNull());
      expect(lower.dismissals()).toBe(0);
      expect(document.activeElement).toBe(trigger);

      expect(dismissTopTransient(reason)).toBe(true);
      expect(lower.dismissals()).toBe(1);
    } finally {
      lower.unregister();
      dispose();
    }
  });

  it("unregisters after backdrop close and component disposal", async () => {
    const lower = lowerSentinel("calendar-cleanup-lower");
    const first = mountCalendar();
    const trigger = first.root.querySelector<HTMLButtonElement>('button[title="Go to date"]')!;
    trigger.click();
    await vi.waitFor(() => expect(first.root.querySelector(".dp-overlay")).not.toBeNull());
    first.root.querySelector<HTMLElement>(".dp-overlay")!.click();
    await vi.waitFor(() => expect(first.root.querySelector(".calendar-jump-pop")).toBeNull());
    expect(dismissTopTransient("escape")).toBe(true);
    expect(lower.dismissals()).toBe(1);

    trigger.click();
    await vi.waitFor(() => expect(first.root.querySelector(".calendar-jump-pop")).not.toBeNull());
    first.dispose();
    expect(dismissTopTransient("back")).toBe(true);
    expect(lower.dismissals()).toBe(2);
    lower.unregister();
  });

  it("keeps two mounted calendars as independent owners", async () => {
    const lower = lowerSentinel("calendar-instances-lower");
    const { root, dispose } = mountCalendar(2);
    try {
      const triggers = Array.from(root.querySelectorAll<HTMLButtonElement>('button[title="Go to date"]'));
      triggers[0].click();
      triggers[1].click();
      await vi.waitFor(() => expect(root.querySelectorAll(".calendar-jump-pop")).toHaveLength(2));

      expect(dismissTopTransient("escape")).toBe(true);
      await vi.waitFor(() => expect(root.querySelectorAll(".calendar-jump-pop")).toHaveLength(1));
      expect(lower.dismissals()).toBe(0);
      expect(document.activeElement).toBe(triggers[1]);

      expect(dismissTopTransient("back")).toBe(true);
      await vi.waitFor(() => expect(root.querySelectorAll(".calendar-jump-pop")).toHaveLength(0));
      expect(lower.dismissals()).toBe(0);
      expect(document.activeElement).toBe(triggers[0]);

      expect(dismissTopTransient("escape")).toBe(true);
      expect(lower.dismissals()).toBe(1);
    } finally {
      lower.unregister();
      dispose();
    }
  });
});
