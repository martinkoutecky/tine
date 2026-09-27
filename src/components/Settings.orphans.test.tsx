import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { closeSettings, openSettings } from "../ui";

const flush = vi.hoisted(() => vi.fn());
vi.mock("../document", async (importOriginal) => ({
  ...await importOriginal<typeof import("../document")>(), flushAll: flush,
}));

import { Settings } from "./Settings";

afterEach(() => { closeSettings(); vi.restoreAllMocks(); document.body.innerHTML = ""; });

it("does not scan orphan assets when pending edits fail to flush", async () => {
  flush.mockResolvedValue(false);
  const scan = vi.spyOn(backend(), "listOrphanAssets").mockResolvedValue([]);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("files");
    const button = [...root.querySelectorAll("button")].find((node) => node.textContent?.includes("Scan for orphans"));
    expect(button).toBeDefined();
    button!.click();
    await vi.waitFor(() => expect(flush).toHaveBeenCalled());
    expect(scan).not.toHaveBeenCalled();
  } finally { dispose(); }
});
