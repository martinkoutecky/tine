import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { renameOrMergePage } from "../graph";
import { resetStore } from "../document";
import { setToasts, toasts } from "../toasts";
import { Toasts } from "./Toasts";
import { showRewriteRefusal } from "../renameRefusal";
import { invalidateBinding } from "../binding";
import { setActiveLocale } from "../i18n";

let dispose: () => void;
beforeEach(() => { dispose = render(() => <Toasts />, document.body); });
afterEach(() => { dispose(); document.body.innerHTML = ""; setToasts([]); resetStore(); vi.restoreAllMocks(); });
const clickSave = () => (document.querySelector(".toast-action:not(.toast-copy)") as HTMLButtonElement).click();
const text = () => document.body.textContent ?? "";

const refusal = (guarantee: "P2" | "P3", file = "pages/referrer.md") => ({ refused: { kind: "checkRefused", guarantee, file } });
const done = { outcome: "renamed" as const, renamed: [], touched: [] };

it("explains a first-save refusal and waits for a click before saving and repeating the same rename", async () => {
  vi.spyOn(backend(), "renameOffer").mockResolvedValue({ kind: "rename" });
  const rename = vi.spyOn(backend(), "renamePage").mockRejectedValueOnce(refusal("P3")).mockResolvedValueOnce(done);
  const recovered = vi.fn();
  await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" }, undefined, recovered);
  expect(text()).toMatch(/Tine would change the layout of pages\/referrer.md/);
  expect(toasts()[0].sticky).toBe(true);
  expect(rename).toHaveBeenCalledTimes(1);
  expect(text()).toContain("Save it and rename");
  clickSave();
  await vi.waitFor(() => expect(rename).toHaveBeenCalledTimes(2));
  await vi.waitFor(() => expect(recovered).toHaveBeenCalledWith("renamed"));
  expect(rename).toHaveBeenLastCalledWith("Old", "New", "rename-page", "pages/Old.md", undefined, [], "pages/referrer.md");
});

it("another refused file needs another click, with the original merge choice retained", async () => {
  vi.spyOn(backend(), "renameOffer").mockResolvedValue({ kind: "merge", into: "pages/New.md", name: "New", journal: false });
  const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  const rename = vi.spyOn(backend(), "renamePage").mockRejectedValueOnce(refusal("P3")).mockRejectedValueOnce(refusal("P3", "pages/second.md")).mockResolvedValueOnce(done);
  await renameOrMergePage("Old", "New");
  clickSave();
  await vi.waitFor(() => expect(text()).toMatch(/layout of pages\/second.md/));
  expect(rename).toHaveBeenCalledTimes(2);
  clickSave();
  await vi.waitFor(() => expect(rename).toHaveBeenCalledTimes(3));
  expect(rename).toHaveBeenLastCalledWith("Old", "New", "rename-page", undefined, "pages/New.md", [], "pages/second.md");
  expect(confirm).toHaveBeenCalledTimes(1);
});

it("a failed save is shown and never automatically retried", async () => {
  vi.spyOn(backend(), "renameOffer").mockResolvedValue({ kind: "rename" });
  const rename = vi.spyOn(backend(), "renamePage").mockRejectedValueOnce(refusal("P3")).mockRejectedValueOnce("disk full");
  await renameOrMergePage("Old", "New");
  clickSave();
  await vi.waitFor(() => expect(text()).toMatch(/disk full/));
  expect(rename).toHaveBeenCalledTimes(2);
  expect(text()).not.toContain("Save it and rename");
});

it("blames Tine for its own failed check and offers no retry", async () => {
  vi.spyOn(backend(), "renameOffer").mockResolvedValue({ kind: "rename" });
  vi.spyOn(backend(), "renamePage").mockRejectedValueOnce(refusal("P2"));
  await renameOrMergePage("Old", "New");
  expect(text()).toMatch(/Tine stopped because its own check failed for pages\/referrer.md/);
  expect(text()).toMatch(/Nothing was changed.*Please report this bug/);
  expect(text()).not.toContain("Save it and rename");
});

it.each([
  ["headerChildren", "blocks under the header in pages/Old.md would lose their parent. Move them out from under the header"],
  ["hiddenOccurrence", "references in the page properties of pages/Old.md would stop working when moved into the merged page. Move those references into ordinary blocks"],
  ["survivorIdentity", "the title in the first block of pages/Old.md would give the remaining page a different name. Save that file once with its intended page title"],
])("explains %s and a legal next step", (kind, explanation) => {
  expect(showRewriteRefusal({ refused: { kind, file: "pages/Old.md" } })).toBe(true);
  expect(text()).toContain(explanation);
  expect(text()).toContain("Nothing was merged.");
  expect(text()).not.toContain("Save it and rename");
  expect(toasts()[0].sticky).toBe(true);
});

it("the recovery action cannot run in a replacement graph", async () => {
  vi.spyOn(backend(), "renameOffer").mockResolvedValue({ kind: "rename" });
  const rename = vi.spyOn(backend(), "renamePage").mockRejectedValueOnce(refusal("P3"));
  await renameOrMergePage("Old", "New");
  invalidateBinding();
  clickSave();
  await Promise.resolve();
  expect(rename).toHaveBeenCalledTimes(1);
});

it("dedupes identical blocking refusals and translates them live", async () => {
  const retry = vi.fn();
  showRewriteRefusal(refusal("P3"), retry);
  showRewriteRefusal(refusal("P3"), retry);
  expect(toasts()).toHaveLength(1);
  try {
    await setActiveLocale("zh-CN");
    expect(text()).toContain("首次保存 pages/referrer.md");
    expect(text()).toContain("保存文件并重命名");
  } finally { await setActiveLocale("en"); }
  expect(retry).not.toHaveBeenCalled();
});
