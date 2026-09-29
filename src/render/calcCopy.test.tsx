import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { CalcBlock } from "./body";
import { initParser } from "./parse";
import { For } from "solid-js";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc, pageByName } from "../document/model";
import { startEditing } from "../editorController";
import { Block } from "../components/Block";
import { clearClipboardPayload } from "../clipboard";
import { backend } from "../backend";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  clearClipboardPayload();
  resetStore();
  document.body.innerHTML = "";
});

function mount(src: string): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <CalcBlock src={src} />, root);
  return { root, dispose };
}

describe("CalcBlock copy button (GH #228)", () => {
  it("offers a copy button only on successful result lines, not on errors or blanks", () => {
    const src = "1 + 1\n2 + 2\nbad /\n# comment\n:hex";
    const { root, dispose } = mount(src);

    const rows = [...root.querySelectorAll(".calc-out")];
    expect(rows).toHaveLength(5);

    const copyBtns = rows.map((r) => r.querySelector("button.copy-btn"));
    expect(copyBtns[0]).not.toBeNull(); // 1 + 1 → 2
    expect(copyBtns[1]).not.toBeNull(); // 2 + 2 → 4
    expect(copyBtns[2]).toBeNull();     // error
    expect(copyBtns[3]).toBeNull();     // comment (output null)
    expect(copyBtns[4]).toBeNull();     // :hex directive (output null)

    dispose();
  });

  it("copies the displayed result text on click via the clipboard facade", () => {
    const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue();
    const { root, dispose } = mount("1 + 1\n2 + 2");

    const btns = root.querySelectorAll<HTMLButtonElement>(".calc-out button.copy-btn");
    expect(btns).toHaveLength(2);
    btns[1]!.click();

    expect(writeText).toHaveBeenCalledWith("4");

    dispose();
  });

  it("copy button click does not mutate the block or create an empty row", () => {
    const { root, dispose } = mount("3 + 4");
    vi.spyOn(backend(), "writeText").mockResolvedValue();
    const before = root.querySelectorAll(".calc-out").length;
    const btn = root.querySelector<HTMLButtonElement>(".calc-out button.copy-btn")!;
    btn.click();
    expect(root.querySelectorAll(".calc-out")).toHaveLength(before);
    dispose();
  });

  it("the live editor's result column offers the same copy button and leaves the block untouched", () => {
    const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue();
    const raw = "```calc\n1 + 1\nbad /\n```";
    loadSingle({ name: "Calc", kind: "page", title: "Calc", pre_block: null, blocks: [{ id: "calc", raw, collapsed: false, children: [] }] });
    startEditing("calc", 0);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <For each={pageByName("Calc")?.roots ?? []}>{(id) => <Block id={id} />}</For>, root);
    try {
      const column = root.querySelector(".calc-results")!;
      expect(column).not.toBeNull();
      const btns = column.querySelectorAll<HTMLButtonElement>(".calc-out button.copy-btn");
      expect(btns).toHaveLength(1); // the error line offers none
      const editor = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
      const before = editor.value;
      btns[0]!.click();
      expect(writeText).toHaveBeenCalledWith("2");
      expect(editor.value).toBe(before);
      expect(doc.byId.calc.raw).toBe(raw);
    } finally {
      dispose();
    }
  });
});
