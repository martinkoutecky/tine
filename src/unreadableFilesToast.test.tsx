// GH #727 (discussion): the unreadable-files toast used to say "the file list is
// in the debug log", but the log is opt-in, so the user could not find the
// broken files. The toast names them; Copy carries the full list.
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "./backend";
import { setToasts, toasts } from "./toasts";
import { Toasts } from "./components/Toasts";

vi.mock("./binding", async (original) => ({ ...(await original<typeof import("./binding")>()), captureBinding: () => ({ backendGeneration: 1 }) }));
import { reportUnreadableFiles } from "./uiFailure";

let dispose: () => void = () => {};
beforeEach(() => {
  setToasts([]);
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <Toasts />, host);
});
afterEach(() => { dispose(); vi.restoreAllMocks(); document.body.innerHTML = ""; setToasts([]); });

it("names a few files in the toast, counts the rest, and Copy lists every file", async () => {
  const files = Array.from({ length: 8 }, (_, i) => `pages/Bad${i}.md`);
  reportUnreadableFiles(files);
  const [toast] = toasts();
  expect(toast.kind).toBe("error");
  expect(toast.sticky).toBe(true);
  for (const name of files.slice(0, 5)) expect(toast.message).toContain(name);
  for (const name of files.slice(5)) expect(toast.message).not.toContain(name);
  expect(toast.message).toContain("and 3 more");
  expect(toast.message).not.toMatch(/debug log/i);
  expect(document.querySelector(".toast-msg")!.textContent).toBe(toast.message);
  const write = vi.spyOn(backend(), "writeText").mockResolvedValue();
  const copy = [...document.querySelectorAll(".toast button")].find((b) => b.textContent?.trim() === "Copy") as HTMLElement;
  copy.click();
  await vi.waitFor(() => expect(write).toHaveBeenCalled());
  const copied = write.mock.calls[0][0] as string;
  for (const name of files) expect(copied).toContain(name);
});

it("a short list is named in full with no 'more' tail", () => {
  reportUnreadableFiles(["pages/Bad.md: stream did not contain valid UTF-8"]);
  expect(toasts().map((t) => t.message)).toEqual([
    "Some page files couldn't be read: pages/Bad.md: stream did not contain valid UTF-8. The rest of the graph is available.",
  ]);
});
