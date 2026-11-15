import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ExportModal } from "./ExportModal";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { closeExportModal, openExportNodesModal } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";
import type { BlockDto, RefGroup } from "../types";
import { publishedBackend, type PublishedSnapshot } from "../publishedBackend";
import { ReferenceExportChooser } from "./ReferenceExportChooser";

beforeAll(initParser);
afterEach(() => {
  closeExportModal();
  clearTransientLayersForTest();
  resetStore();
  document.body.innerHTML = "";
  localStorage.clear();
  vi.restoreAllMocks();
});

it.each(["embed", "query", "tine-query"].flatMap(macro => [false, true].map(reply => ({ macro, reply }))))("$macro exports a detached thread block (reply=$reply) labelled, and omits it on request", async ({ macro, reply }) => {
  const uuid = "6a1b2c3d-0000-4000-8000-000000000009";
    const block: BlockDto & { comment: boolean } = {
      id: uuid, raw: reply ? "REPLYMARKER" : "COMMENTMARKER\ntine.quote:: Body",
      comment: true, collapsed: false, children: [],
    };
    const group = { page: "Source", kind: "page" as const, blocks: [block] };
    const query = macro === "tine-query" ? "task = 'TODO'" : "(task TODO)";
    const key = JSON.stringify([macro, [query]]);
    const previewBlock = vi.spyOn(backend(), "previewBlock").mockResolvedValue({ group, truncated: 0 });
    const exportQuery = vi.spyOn(backend(), "exportQuerySubtrees").mockResolvedValue({
      results: [{ key, groups: [group], shown: 1, total: 1, omitted_nodes: 0 }], omitted_queries: 0,
    });
    const getPage = vi.spyOn(backend(), "getPage");
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <ExportModal />, root);
    try {
      const raw = macro === "embed" ? `{{embed ((${uuid}))}}` : `{{${macro} ${query}}}`;
      openExportNodesModal([{ raw: "Body", children: [{ raw, children: [] }] }], 2);
      const preview = () => root.querySelector<HTMLTextAreaElement>(".export-preview")?.value ?? "";
      await vi.waitFor(() => expect(preview()).toContain(reply ? "REPLYMARKER" : "COMMENTMARKER"));
      expect(preview()).toContain("Comment");
      if (macro === "embed") {
        const source = [...root.querySelectorAll<HTMLButtonElement>("button")]
          .find(button => button.textContent?.trim() === "Markdown")!;
        source.click();
        await vi.waitFor(() => expect(preview()).toContain("tine.quote"));
        expect(preview()).toContain(reply ? "REPLYMARKER" : "COMMENTMARKER");
      }
      const checkbox = [...root.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')]
        .find(input => input.closest("label")?.textContent?.includes("Include comments"))!;
      expect(checkbox.checked).toBe(true);
      checkbox.click();
      await vi.waitFor(() => expect(preview()).not.toMatch(/COMMENTMARKER|REPLYMARKER/));
      expect(preview()).toContain("Body");
      expect(getPage).not.toHaveBeenCalled();
      expect(macro === "embed" ? previewBlock : exportQuery).toHaveBeenCalledTimes(1);
    } finally { closeExportModal(); dispose(); root.remove(); vi.restoreAllMocks(); }
});

const ROOT_ID = "6a1b2c3d-0000-4000-8000-0000000000c1";
const REPLY_ID = "6a1b2c3d-0000-4000-8000-0000000000c2";
const BODY_ID = "6a1b2c3d-0000-4000-8000-0000000000c3";

/** Mount the export modal, check the default label and the Include comments
 * switch on the previewed text, and return nothing (the outcome is the text). */
async function expectLabelledThenOmitted(open: () => void, markers: string[], kept: string) {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <ExportModal />, root);
  try {
    open();
    const preview = () => root.querySelector<HTMLTextAreaElement>(".export-preview")?.value ?? "";
    await vi.waitFor(() => expect(preview()).toContain(markers[0]));
    for (const marker of markers) expect(preview()).toMatch(new RegExp(`\\[Comment[^\\]]*\\] ${marker}`));
    expect(preview()).toContain(kept);
    const checkbox = [...root.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')]
      .find(input => input.closest("label")?.textContent?.includes("Include comments"))!;
    checkbox.click();
    await vi.waitFor(() => expect(preview()).not.toMatch(new RegExp(markers.join("|"))));
    expect(preview()).toContain(kept);
  } finally { closeExportModal(); dispose(); root.remove(); }
}

// Iif8k: the published reader answers embed previews from its snapshot; a
// comment root or unquoted reply must keep its thread membership there.
it.each([false, true])("a published reader embed of a comment (reply=%s) is labelled and omitted on request", async (reply) => {
  const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });
  const snapshot: PublishedSnapshot = {
    schema: 1, name: "Reader", exported_at: "", home: "Source",
    pages: [{ path: "pages/Source.md", name: "Source", title: "Source", kind: "page", pre_block: null, read_only: true, blocks: [
      block("body", `Body\nid:: ${BODY_ID}`, [
        block("root", `COMMENTMARKER\nid:: ${ROOT_ID}\ntine.quote:: Body`, [block("reply", `REPLYMARKER\nid:: ${REPLY_ID}`)]),
      ]),
    ] }],
    entries: [{ name: "Source", kind: "page", date_key: null, path: "pages/Source.md" }],
    backlinks: {}, block_ref_counts: {}, aliases: [], icons: {}, queries: [],
  };
  const reader = publishedBackend(async () => snapshot);
  vi.spyOn(backend(), "previewBlock").mockImplementation((uuid, max) => reader.previewBlock(uuid, max));
  const embed = `{{embed ((${reply ? REPLY_ID : ROOT_ID}))}}`;
  await expectLabelledThenOmitted(
    () => openExportNodesModal([{ raw: "Reader host", children: [{ raw: embed, children: [] }] }], 2),
    [reply ? "REPLYMARKER" : "COMMENTMARKER"], "Reader host");
  // An embedded body block keeps its ordinary text; membership is not invented.
  const body = await reader.previewBlock(BODY_ID, 10);
  expect(body?.group.blocks[0].comment).toBeUndefined();
});

// Iif8k: reference rows are shallow; the native reference producer supplies
// thread membership, and the chooser's Copy/Export must use it.
it("reference-panel Copy/Export labels comment rows and omits them on request", async () => {
  const groups: RefGroup[] = [{ page: "Source", kind: "page", blocks: [
    { id: "root", raw: "COMMENTMARKER [[Target]]\ntine.quote:: Body", comment: true, breadcrumb: ["Body"], collapsed: false, children: [] },
    { id: "reply", raw: "REPLYMARKER [[Target]]", comment: true, breadcrumb: ["Body", "COMMENTMARKER"], collapsed: false, children: [] },
    { id: "plain", raw: "ORDINARY [[Target]]", breadcrumb: ["Body"], collapsed: false, children: [] },
  ] }];
  const host = document.createElement("div");
  document.body.append(host);
  const disposeChooser = render(() => <ReferenceExportChooser subject="Linked References" groups={groups} onClose={() => {}} />, host);
  try {
    await expectLabelledThenOmitted(() => host.querySelector<HTMLButtonElement>(".export-btn-primary")!.click(), ["COMMENTMARKER", "REPLYMARKER"], "ORDINARY");
  } finally { disposeChooser(); host.remove(); }
});
