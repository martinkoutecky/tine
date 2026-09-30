import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { QuickSwitcher } from "./QuickSwitcher";
import { backend } from "../backend";
import { closeSwitcher, openSwitcher } from "../ui";
import { resetStore } from "../document";
import { setToasts, toasts } from "../toasts";

afterEach(() => { closeSwitcher(); resetStore(); vi.restoreAllMocks(); setToasts([]); document.body.innerHTML = ""; });
it("retries a failed search in place, without a red toast or a false empty result", async () => {
  setToasts([]);
  const search = vi.spyOn(backend(), "runGraphSearch").mockRejectedValueOnce(new Error("search failed"))
    .mockResolvedValue({ hits: [], diagnostics: [], explanation: { branches: [] }, has_more: { pages: false, blocks: false }, cancelled: false });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QuickSwitcher />, root);
  openSwitcher();
  try {
    const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
    input.value = "missing"; input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await vi.waitFor(() => expect(root.querySelector(".resource-failure")).not.toBeNull());
    expect(root.textContent).not.toContain("No matched results");
    const retry = root.querySelector<HTMLButtonElement>(".resource-failure-retry");
    expect(retry).not.toBeNull();
    retry!.click();
    await vi.waitFor(() => expect(search).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(root.querySelector(".resource-failure")).toBeNull());
    expect(input.value).toBe("missing");
    expect(toasts()).toEqual([]);
  } finally { dispose(); }
});
