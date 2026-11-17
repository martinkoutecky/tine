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
  expect(text()).toMatch(/Nothing was renamed or merged.*Please report this bug/);
  expect(text()).not.toContain("Save it and rename");
});

it("localizes a duplicate-title refusal from the real rename question await in Chinese", async () => {
  // The command test seeds both files with title:: A; this is its wire answer.
  const offer = vi.spyOn(backend(), "renameOffer").mockRejectedValue({ refused: {
    kind: "duplicateFiles", files: ["pages/one.md", "pages/two.md"],
  } });
  const rename = vi.spyOn(backend(), "renamePage");
  try {
    await setActiveLocale("zh-CN");
    expect(await renameOrMergePage("A", "B")).toBe("cancelled");
    expect(offer).toHaveBeenCalledWith("A", "B", undefined);
    expect(text()).toContain("这些文件声明了同一个页面");
    expect(text()).toContain("pages/one.md, pages/two.md");
    expect(text()).not.toContain("[object Object]");
    expect(toasts()[0].sticky).toBe(true);
    expect(rename).not.toHaveBeenCalled();
  } finally { await setActiveLocale("en"); }
});

it.each([
  ["bodyLayout", "Tine cannot preserve the block structure", "Align the top-level blocks"],
  ["headerProseOverlap", "as both properties and prose", "Remove blank lines before or between the property lines"],
  ["indentedBody", "the first block of pages/source.md is indented", "Align the top-level blocks"],
  ["directiveEscape", "contains a #+END_QUOTE line outside a quote", "Move that line into a quote"],
  ["aliasSpelling", "the alias declaration “alias:: [[C\\#]]”", "plain alias names"],
])("explains the ordinary-file %s limitation in both languages", async (kind, explanation, next) => {
  showRewriteRefusal({ refused: { kind, file: "pages/source.md", alias: "alias:: [[C\\#]]" } });
  expect(text()).toContain(explanation);
  expect(text()).toContain(next);
  expect(text()).toContain("pages/source.md");
  expect(text()).toContain("Nothing was merged.");
  expect(text()).not.toMatch(/bug|own check/);
  try {
    await setActiveLocale("zh-CN");
    expect(text()).toContain("无法合并");
    expect(text()).toContain("请");
    expect(text()).toContain("pages/source.md");
    if (kind === "headerProseOverlap") {
      expect(text()).toContain("同时读取为属性和正文");
      expect(text()).toContain("请删除该文件顶部属性行之前或之间的空白行");
    }
  } finally { await setActiveLocale("en"); }
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

it.each([
  [{ kind: "differentFormats", source: "pages/A.md", target: "pages/B.org" }, "pages/A.md and pages/B.org use different formats"],
  [{ kind: "mergeConflictMarkers", file: "pages/A.md" }, "Resolve the conflict markers in pages/A.md"],
  [{ kind: "sameFile", file: "pages/A.md" }, "pages/A.md is both the source and the remaining file"],
  [{ kind: "staleMerge", file: "pages/B.md" }, "pages/B.md no longer holds the name you chose"],
  [{ kind: "unindexed", file: "pages/A.md" }, "Tine cannot read pages/A.md safely"],
  [{ kind: "readOnlyOrg", file: "pages/A.org" }, "Tine cannot save pages/A.org without changing its content"],
  [{ kind: "staleSource", file: "pages/A.md" }, "pages/A.md no longer identifies the page you chose"],
  [{ kind: "titleIdentity", file: "pages/A.md" }, "changing the references inside the title of pages/A.md would give the page a different name"],
  [{ kind: "duplicateFiles", files: ["pages/A.md", "pages/B.md"] }, "these files claim the same page: pages/A.md, pages/B.md"],
])("localizes the remaining refusal %# and keeps it until dismissed", async (reason, explanation) => {
  expect(showRewriteRefusal({ refused: reason })).toBe(true);
  expect(text()).toContain(explanation);
  expect(text()).not.toContain("Save it and rename");
  expect(toasts()[0].sticky).toBe(true);
  try {
    await setActiveLocale("zh-CN");
    expect(text()).toContain("无法");
    expect(text()).toContain("请");
    expect(text()).toContain("pages/");
  } finally { await setActiveLocale("en"); }
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
