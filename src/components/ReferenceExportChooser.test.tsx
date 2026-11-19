import { afterEach, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { closeExportModal, exportModal } from "../ui";
import type { RefGroup } from "../types";
import { ReferenceExportChooser } from "./ReferenceExportChooser";

afterEach(() => {
  closeExportModal();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

it("exports the opening selection snapshot in each source page's format", async () => {
  const block = (id: string) => ({ id, raw: `TODO ${id}`, collapsed: false, children: [] });
  vi.spyOn(backend(), "getPage").mockResolvedValue({
    id: "pages/Org Source.org", name: "Org Source", title: "Org Source", kind: "page",
    pre_block: null, format: "org", blocks: [block("a"), block("b")],
  });
  const [groups, setGroups] = createSignal<RefGroup[]>([
    { page: "Org Source", kind: "page", blocks: [block("a"), block("b")] },
  ]);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <ReferenceExportChooser subject="Linked References" groups={groups()} onClose={() => {}} />, root);
  try {
    const checks = root.querySelectorAll<HTMLInputElement>('input[type="checkbox"]');
    checks[1].click();
    setGroups([{ page: "Org Source", kind: "page", blocks: [block("replacement")] }]);
    expect(root.querySelectorAll<HTMLInputElement>('input[type="checkbox"]').length).toBe(2);
    expect(root.querySelector(".export-count")?.textContent).toBe("1 of 2");
    root.querySelector<HTMLButtonElement>(".export-btn-primary")!.click();
    await vi.waitFor(() => {
      const request = exportModal();
      expect(request && "nodes" in request ? request.nodes : null).toEqual([
        { raw: "Org Source", format: "org", children: [{ id: "a", hasParent: false, raw: "TODO a", format: "org", children: [] }] },
      ]);
    });
  } finally { dispose(); }
});
