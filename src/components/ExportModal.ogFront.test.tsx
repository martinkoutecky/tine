import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ExportModal } from "./ExportModal";
import { backend } from "../backend";
import { resetStore } from "../document";
import { setDoc } from "../document/model";
import { closeExportModal, openExportModal } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";

afterEach(() => {
  closeExportModal(); clearTransientLayersForTest(); resetStore();
  document.body.innerHTML = ""; vi.restoreAllMocks(); localStorage.clear();
});

it.each(["self", "mutual", "benign"])("exports %s embeds without runaway callback recursion (OG-B-FRONT)", async kind => {
  const raw = "Start {{embed [[A]]}} End";
  setDoc({byId: {root: {id: "root", raw, parent: null, page: "P", collapsed: false, children: []}},
    pages: [{name: "P", kind: "page", title: "P", preBlock: null, roots: ["root"], format: "md", readOnly: false, guide: false}],
    feed: ["P"], loaded: true});
  vi.spyOn(backend(), "getPage").mockResolvedValue({id: "pages/A.md", name: "A", kind: "page", title: "A", pre_block: null,
    blocks: [{id: "a", raw: kind === "self" ? "A {{embed [[A]]}}" : kind === "mutual" ? "A {{embed [[B]]}}" : "A " + "content ".repeat(2000), collapsed: false, children: []}]});
  // Both keys are warmed from the selected forest for a mutual cycle.
  if (kind === "mutual") {
    setDoc("byId", "root", "raw", raw + " {{embed [[B]]}}");
    vi.mocked(backend().getPage).mockImplementation(async name => ({id: `pages/${name}.md`, name, kind: "page", title: name, pre_block: null,
      blocks: [{id: name, raw: `${name} {{embed [[${name === "A" ? "B" : "A"}]]}}`, collapsed: false, children: []}]}));
  }
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <ExportModal />, root);
  try {
    openExportModal(["root"]);
    await vi.waitFor(() => {
      const text = root.querySelector<HTMLTextAreaElement>(".export-preview")!.value;
      expect(text).toContain("Start"); expect(text).toContain("End");
      expect(text).toContain(kind === "benign" ? "content" : "embed expansion omitted");
    });
  } finally { dispose(); }
});
