import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { setToasts, toasts, dismissToast } from "./toasts";
import {
  DEFENDER_HINT_ACTION,
  DEFENDER_HINT_TEXT,
  exclusionResultMessage,
  maybeShowDefenderHint,
} from "./defenderHint";

beforeEach(() => setToasts([]));
afterEach(() => {
  vi.restoreAllMocks();
  setToasts([]);
});

describe("Windows Defender hint (GH #623)", () => {
  it("shows nothing unless the backend says so", async () => {
    vi.spyOn(backend(), "defenderHint").mockResolvedValue({ show: false });
    await maybeShowDefenderHint();
    expect(toasts()).toHaveLength(0);
  });

  it("an unreadable answer shows no hint and no error", async () => {
    vi.spyOn(backend(), "defenderHint").mockRejectedValue(new Error("boom"));
    await maybeShowDefenderHint();
    expect(toasts()).toHaveLength(0);
  });

  it("shows a sticky hint whose click, and only whose click, asks for the exclusion", async () => {
    vi.spyOn(backend(), "defenderHint").mockResolvedValue({ show: true });
    const add = vi.spyOn(backend(), "addDefenderExclusion").mockResolvedValue({ outcome: "added" });
    const dismiss = vi.spyOn(backend(), "dismissDefenderHint").mockResolvedValue();
    await maybeShowDefenderHint();
    expect(toasts()).toHaveLength(1);
    expect(toasts()[0].message).toBe(DEFENDER_HINT_TEXT);
    expect(toasts()[0].sticky).toBe(true);
    expect(toasts()[0].action?.label).toBe(DEFENDER_HINT_ACTION);
    // Showing it changed nothing.
    expect(add).not.toHaveBeenCalled();
    const hint = toasts()[0];
    hint.action!.run();
    dismissToast(hint.id); // the toast host dismisses after an action
    await vi.waitFor(() => expect(toasts().some((t) => t.kind === "success")).toBe(true));
    expect(add).toHaveBeenCalledTimes(1);
    // Choosing the action is not a dismissal; the backend records a success.
    expect(dismiss).not.toHaveBeenCalled();
  });

  it("closing the hint records the per-graph dismissal", async () => {
    vi.spyOn(backend(), "defenderHint").mockResolvedValue({ show: true });
    const dismiss = vi.spyOn(backend(), "dismissDefenderHint").mockResolvedValue();
    await maybeShowDefenderHint();
    dismissToast(toasts()[0].id);
    expect(dismiss).toHaveBeenCalledTimes(1);
  });

  it("reports a declined prompt and a failure in words, and never as success", () => {
    expect(exclusionResultMessage({ outcome: "declined" }).kind).toBe("info");
    const failed = exclusionResultMessage({ outcome: "failed", code: 1, message: "Managed by your organization." });
    expect(failed.kind).toBe("error");
    expect(failed.text).toContain("Managed by your organization.");
    expect(exclusionResultMessage({ outcome: "added" }).kind).toBe("success");
  });
});
