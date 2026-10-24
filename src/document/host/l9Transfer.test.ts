import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { initParser } from "../../render/parse";
import { backend } from "../../backend";
import { beginPageHeaderEdit, finishPageHeaderEdit, flushAll, pageByName, resetStore, setRaw } from "..";
import { doc } from "../model";
import { loadSingle } from "../workingSet";
import { pageToDto } from "../convert";
import { moveBlock } from "../edits/moves";
import { bindTestHost, submittedPages } from "./wiring.test.support";
import { capturePages, persistTransfer } from "./wiring";
import { setDoc } from "../model";
import { toasts, setToasts } from "../../toasts";

beforeAll(() => initParser());
afterEach(() => { vi.restoreAllMocks(); resetStore(); });

it.each(["Src", "Dst"])("L9-07: refuses a transfer with an invalid %s header without losing the block", async (invalid) => {
  resetStore();
  for (const name of ["Src", "Dst"]) loadSingle({ name, kind: "page", title: name,
    id: `pages/${name}.md`, rev: name, pre_block: "tags:: x", format: "md",
    blocks: [{ id: name, raw: name === "Src" ? "the moved block" : "destination", collapsed: false, children: [] }] });
  await bindTestHost();
  const header = beginPageHeaderEdit(invalid)!;
  setRaw(header, "tags:: x\nhalf typed", { timetracking: false });
  finishPageHeaderEdit(header);
  expect(pageToDto(invalid)).toBeNull();
  const submit = vi.spyOn(backend(), "pageSubmit");
  const move = vi.spyOn(backend(), "pageMove");
  await moveBlock("Src", null, pageByName("Dst")!.roots.length, "Dst");
  await flushAll();
  expect(doc.byId.Src.page).toBe("Src");
  expect(doc.byId[header].raw).toBe("tags:: x\nhalf typed");
  expect(move).not.toHaveBeenCalled();
  expect(submittedPages(submit, "Src").every(dto => JSON.stringify(dto).includes("the moved block"))).toBe(true);
});

it("L9-07: rolls back an applied transfer if an endpoint cannot serialize", async () => {
  resetStore();
  for (const name of ["Src", "Dst"]) loadSingle({ name, kind: "page", title: name,
    id: `pages/${name}.md`, rev: name, pre_block: "tags:: x", format: "md",
    blocks: [{ id: name, raw: name, collapsed: false, children: [] }] });
  await bindTestHost();
  const header = beginPageHeaderEdit("Dst")!;
  setRaw(header, "tags:: x\nunfinished", { timetracking: false });
  const before = capturePages(["Src", "Dst"]);
  setDoc("pages", p => p.name === "Src", "roots", []);
  setDoc("pages", p => p.name === "Dst", "roots", roots => [...roots, "Src"]);
  setDoc("byId", "Src", "page", "Dst");
  const submit = vi.spyOn(backend(), "pageSubmit");
  setToasts([]);
  expect(await persistTransfer(before, "move-blocks", [["Src", "Dst"]])).toBe(false);
  expect(toasts().some(toast => toast.kind === "error")).toBe(true);
  expect(doc.byId.Src.page).toBe("Src");
  expect(doc.byId[header].originatedFromPageHeader).toBe(true);
  expect(doc.byId[header].raw).toBe("tags:: x\nunfinished");
  await flushAll();
  expect(submittedPages(submit, "Src").every(dto => dto.blocks.some(b => b.raw === "Src"))).toBe(true);
});
