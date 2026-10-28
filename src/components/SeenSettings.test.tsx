import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import type { SeenPageChanges } from "../backendTypes";
import { setToasts } from "../toasts";
import { noteSeenAnswer, resetSeenForTests } from "../seen/seenState";
import { SeenSettings } from "./SeenSettings";

afterEach(() => { vi.restoreAllMocks(); resetSeenForTests(); setToasts([]); document.body.replaceChildren(); });
const answer = (available: boolean) => ({ available, reason: null, blocks: [], removed: [] }) as unknown as SeenPageChanges;

it("Settings shows Mark whole graph seen and Forget seen state only on a graph that can answer", async () => {
  const read = vi.spyOn(backend(), "getPageChanges").mockResolvedValue(answer(false));
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <SeenSettings />, host);
  try {
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
    await Promise.resolve();
    expect(host.querySelector("[data-seen-mark-graph]")).toBeNull();
    expect(host.querySelector("[data-seen-forget]")).toBeNull();
    noteSeenAnswer(true);
    await vi.waitFor(() => expect(host.querySelector("[data-seen-mark-graph]")?.textContent).toBe("Mark whole graph seen"));
    expect(host.querySelector("[data-seen-forget]")?.textContent).toBe("Forget seen state");
    const markGraph = vi.spyOn(backend(), "markGraphSeen").mockResolvedValue(undefined);
    const forget = vi.spyOn(backend(), "forgetSeenState").mockResolvedValue(undefined);
    (host.querySelector("[data-seen-mark-graph]") as HTMLButtonElement).click();
    (host.querySelector("[data-seen-forget]") as HTMLButtonElement).click();
    await vi.waitFor(() => expect(markGraph).toHaveBeenCalledTimes(1));
    await vi.waitFor(() => expect(forget).toHaveBeenCalledTimes(1));
  } finally { dispose(); }
});
