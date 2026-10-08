import { afterEach, expect, it } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { setDoc } from "../document/model";
import { closeExportModal, exportModal } from "../ui";
import type { RefGroup } from "../types";
import { ReferenceExportChooser } from "./ReferenceExportChooser";

afterEach(() => {
  closeExportModal();
  document.body.innerHTML = "";
});

it("exports the opening selection snapshot in each source page's format", () => {
  setDoc({ byId: {}, pages: [{ name: "Org Source", kind: "page", title: "Org Source",
    preBlock: null, roots: [], format: "org", readOnly: false, guide: false }], feed: ["Org Source"], loaded: true });
  const block = (id: string) => ({ id, raw: `TODO ${id}`, collapsed: false, children: [] });
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
    const request = exportModal();
    expect(request && "nodes" in request ? request.nodes : null).toEqual([
      { raw: "Org Source", format: "org", children: [{ raw: "TODO a", format: "org", children: [] }] },
    ]);
  } finally { dispose(); }
});
