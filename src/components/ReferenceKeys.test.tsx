import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { applyChangeSummary } from "../changeSummary";
import { bumpDataRev, bumpGraphEpoch } from "../graphSession";
import { resetStore } from "../document";
import { startEditing, endEdit } from "../editorController";
import type { ChangeSummary } from "../backendTypes";
import type { BlockDto, RefGroup } from "../types";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { TaggedPages } from "./TaggedPages";
import { BlockReferences } from "./BlockReferences";
import { TagTableToggle, TagPageTable } from "./Page";

vi.mock("./LiveRefGroup", () => ({ LiveRefGroup: (props: { blocks: BlockDto[] }) =>
  <div><For each={props.blocks}>{block => <span>{block.raw}</span>}</For></div> }));
vi.mock("../sheet/SheetTable", () => ({ SheetTable: (props: { groups: RefGroup[] }) =>
  <div><For each={props.groups}>{group => <span>{group.blocks[0]?.raw}</span>}</For></div> }));

let rev = 0;
let dispose: (() => void) | undefined;
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
function publish(fields: Partial<ChangeSummary> = {}) {
  applyChangeSummary({ rev: String(++rev), full: false, pages: [], blocks: [], linksTo: [], textTo: [], tagsTo: [], tagTableTo: [], refsTo: [], props: {}, inventoryChanged: false, ...fields });
  bumpDataRev(true);
}
const group = (raw: string): RefGroup => ({ page: "Source", kind: "page", blocks: [{ id: "shown", raw, children: [], collapsed: false }] });
beforeEach(() => { resetStore(); bumpGraphEpoch(); rev = 0; publish(); });
afterEach(() => { endEdit("blur"); dispose?.(); dispose = undefined; vi.restoreAllMocks(); document.body.innerHTML = ""; });

it("a late larger answer cannot auto-collapse references while an edited result is retained", async () => {
  let answer = [group("Retained original result")];
  const read = vi.spyOn(backend(), "getBacklinks").mockImplementation(async () => answer);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <LinkedReferences name="Target" />, root);
  await vi.waitFor(() => expect(root.textContent).toContain("Retained original result"));
  let finish!: (groups: RefGroup[]) => void;
  read.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  publish({ linksTo: ["target"] });
  await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(2));
  startEditing("shown");
  answer = [{ ...group("Larger fresh result"), blocks: Array.from({ length: 101 }, (_, i) => ({
    id: i === 0 ? "shown" : `added-${i}`, raw: `Larger fresh result ${i}`, children: [], collapsed: false,
  })) }];
  finish(answer); await tick(); await tick();
  expect(root.textContent).toContain("Retained original result");
  expect(root.querySelector(".references-count")?.textContent).toBe("1");
  endEdit("blur");
  await vi.waitFor(() => expect(root.querySelector(".references-count")?.textContent).toBe("101"));
});

it.each(["linked", "unlinked", "referrers", "tagged", "table", "toggle"])("%s skips unrelated saves and refreshes related evidence, full and gaps", async reader => {
  let text = "Original answer";
  const method = reader === "linked" ? "getBacklinks" : reader === "unlinked" ? "getUnlinkedRefs"
    : reader === "referrers" ? "getBlockReferrers" : reader === "tagged" ? "taggedPages" : "queryRun";
  const read = vi.spyOn(backend(), method).mockImplementation(async () =>
    (reader === "tagged" ? { anchor: "page", pages: [{ path: "pages/Source.md", name: text, kind: "page", properties: [] }], exceeded: false }
      : reader === "table" || reader === "toggle" ? { anchor: "block", groups: [group(text)], exceeded: false }
      : [group(text)]) as never);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => reader === "linked" ? <LinkedReferences name="Target" />
    : reader === "unlinked" ? <UnlinkedReferences name="Target" />
    : reader === "referrers" ? <BlockReferences id="target-id" />
    : reader === "tagged" ? <TaggedPages name="Target" />
    : reader === "table" ? <TagPageTable pageName="Target" />
    : <TagTableToggle page={{ name: "Target", kind: "page", roots: [] } as never} />, root);
  await vi.waitFor(() => expect(read).toHaveBeenCalled()); await tick(); await tick();
  if (reader === "unlinked") root.querySelector<HTMLElement>(".references-header")!.click();
  read.mockClear();
  publish({ pages: ["unrelated"], blocks: ["unrelated"], linksTo: ["unrelated"] }); await tick();
  expect(read).not.toHaveBeenCalled();
  const related = reader === "unlinked" ? { textTo: ["target"] } : reader === "referrers" ? { refsTo: ["target-id"] }
    : reader === "tagged" ? { tagsTo: ["target"] } : reader === "table" || reader === "toggle" ? { tagTableTo: ["target"] } : { linksTo: ["target"] };
  for (const fields of [related, { full: true }, { gap: true }]) {
    text += " changed";
    if ("gap" in fields) rev += 2;
    publish(fields as Partial<ChangeSummary>);
    await vi.waitFor(() => expect(read).toHaveBeenCalled()); await tick();
    read.mockClear();
  }
  if (reader !== "toggle") {
    const header = root.querySelector<HTMLElement>(".references-header");
    if (header && !root.textContent?.includes(text)) header.click();
    await vi.waitFor(() => expect(root.textContent).toContain(text));
    if (header) {
      header.click(); await tick(); read.mockClear();
      publish(related as Partial<ChangeSummary>);
      if (["linked", "unlinked", "tagged"].includes(reader)) {
        await tick();
        expect(read).not.toHaveBeenCalled();
        expect(root.querySelector('[aria-label="Count may be out of date"]')).not.toBeNull();
        header.click();
        await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
        await tick();
        expect(root.querySelector('[aria-label="Count may be out of date"]')).toBeNull();
      } else await vi.waitFor(() => expect(read).toHaveBeenCalled());
    }
  }
  if (["linked", "unlinked", "referrers", "table", "toggle"].includes(reader)) {
    read.mockClear(); publish({ blocks: ["shown"] });
    await vi.waitFor(() => expect(read).toHaveBeenCalled());
  }
});

it("unlinked registration precedes the first read, retargets, and releases even if disposed while pending", async () => {
  const host = backend();
  const original = host.registerUnlinkedReference;
  const pending: (() => void)[] = [];
  const registration = vi.fn((_id: string, target: string | null, _generation: number) => target
    ? new Promise<void>(resolve => pending.push(resolve)) : Promise.resolve());
  host.registerUnlinkedReference = registration;
  const read = vi.spyOn(host, "getUnlinkedRefs").mockResolvedValue([group("answer")]);
  const [name, setName] = createSignal("Target");
  const root = document.createElement("div"); document.body.append(root);
  try {
    dispose = render(() => <UnlinkedReferences name={name()} />, root);
    await tick(); expect(read).not.toHaveBeenCalled();
    const first = registration.mock.calls[0];
    setName("Other"); await tick();
    expect(registration.mock.calls.some(call => call[1] === "Other")).toBe(true);
    pending[0](); await tick();
    expect(registration).toHaveBeenCalledWith(first[0], null, first[2]);
    expect(read).not.toHaveBeenCalled();
    const second = registration.mock.calls.find(call => call[1] === "Other")!;
    dispose(); dispose = undefined; pending[1](); await tick();
    expect(registration).toHaveBeenCalledWith(second[0], null, second[2]);
    expect(read).not.toHaveBeenCalled();
  } finally {
    host.registerUnlinkedReference = original;
  }
});

it.each(["linked", "unlinked", "referrers", "table"])("%s keeps displayed ancestor text live through a source-page dependency", async reader => {
  let text = "Nested answer";
  const read = vi.spyOn(backend(), reader === "linked" ? "getBacklinks" : reader === "unlinked" ? "getUnlinkedRefs" : reader === "referrers" ? "getBlockReferrers" : "queryRun")
    .mockImplementation(async () => {
      const nested = group(text); nested.blocks[0].breadcrumb = ["Ancestor"];
      return (reader === "table" ? { anchor: "block", groups: [nested], exceeded: false } : [nested]) as never;
    });
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => reader === "linked" ? <LinkedReferences name="Target" />
    : reader === "unlinked" ? <UnlinkedReferences name="Target" />
    : reader === "referrers" ? <BlockReferences id="target-id" /> : <TagPageTable pageName="Target" />, root);
  await vi.waitFor(() => expect(read).toHaveBeenCalled()); await tick(); read.mockClear();
  if (reader === "unlinked") root.querySelector<HTMLElement>(".references-header")!.click();
  publish({ pages: ["unrelated"] }); await tick(); expect(read).not.toHaveBeenCalled();
  text = "Ancestor changed";
  publish({ pages: ["source"], blocks: ["ancestor-not-in-the-shallow-answer"] });
  await vi.waitFor(() => expect(read).toHaveBeenCalled());
});

it.each(["linked", "unlinked", "referrers", "table"])("%s adopts membership removal after a shown member has already changed", async reader => {
  let groups = [group("Present answer")];
  const read = vi.spyOn(backend(), reader === "linked" ? "getBacklinks" : reader === "unlinked" ? "getUnlinkedRefs" : reader === "referrers" ? "getBlockReferrers" : "queryRun")
    .mockImplementation(async () => (reader === "table" ? { anchor: "block", groups, exceeded: false } : groups) as never);
  const related = reader === "linked" ? { linksTo: ["target"] } : reader === "unlinked" ? { textTo: ["target"] }
    : reader === "referrers" ? { refsTo: ["target-id"] } : { tagTableTo: ["target"] };
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => reader === "linked" ? <LinkedReferences name="Target" />
    : reader === "unlinked" ? <UnlinkedReferences name="Target" />
    : reader === "referrers" ? <BlockReferences id="target-id" /> : <TagPageTable pageName="Target" />, root);
  await vi.waitFor(() => expect(read).toHaveBeenCalled()); await tick();
  if (reader === "unlinked") root.querySelector<HTMLElement>(".references-header")?.click();
  await vi.waitFor(() => expect(root.textContent).toContain("Present answer"));
  groups = [group("Edited answer")]; publish({ blocks: ["shown"] });
  await vi.waitFor(() => expect(root.textContent).toContain("Edited answer"));
  groups = []; publish(related);
  await vi.waitFor(() => expect(root.textContent).not.toContain("Edited answer"));
  groups = [group("Restored answer")]; publish(related);
  await vi.waitFor(() => expect(root.textContent).toContain("Restored answer"));
});
