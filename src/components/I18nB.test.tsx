import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { For } from "solid-js";
import { COMMANDS, ADVANCED_BLOCK_COMMANDS, advancedBlockInsertion, filterCommands } from "../editor/autocomplete";
import { DATE_PRESETS } from "../editor/dateExpr";
import { PAGE_PROP_SPECS } from "../editor/properties";
import { SEARCH_SYNTAX } from "../editor/searchQuery";
import { SELECTION_ACTIONS } from "../editor/selectionActions";
import { PdfExportDialog } from "./PdfExportDialog";
import { setPdfExportPage } from "../ui";
import { createTableColumnResize } from "../sheet/tableColumnResize";
import { resetStore } from "../document";
import { setDoc } from "../document/model";

const locale = vi.hoisted(() => ({ set: (_value: boolean) => {} }));
// A test locale decorates every catalog result. This tests display consumption
// even for messages whose zh-CN translation is deliberately outside this lane.
vi.mock("../i18n", async (original) => {
  const actual = await original<typeof import("../i18n")>();
  const { createSignal } = await import("solid-js");
  const [decorated, setDecorated] = createSignal(false);
  locale.set = setDecorated;
  const display = (text: string) => decorated() ? `«${text}»` : text;
  return {
    ...actual,
    t: (...args: Parameters<typeof actual.t>) => display(actual.t(...args)),
    render: (text: Parameters<typeof actual.render>[0]) => display(actual.render(text)),
  };
});

afterEach(() => {
  locale.set(false);
  setPdfExportPage(null);
  resetStore();
  document.body.replaceChildren();
});

it("stored editor labels follow a language switch while English triggers and insertions stay stable", () => {
  const root = document.createElement("div");
  document.body.append(root);
  const upload = COMMANDS.find((command) => command.action === "upload-asset")!;
  const src = ADVANCED_BLOCK_COMMANDS.find((command) => (command.matchLabel ?? command.label) === "Src")!;
  const link = SELECTION_ACTIONS.find((action) => action.id === "link")!;
  const matching = filterCommands("upload");
  const dispose = render(() => <>
    <span>{upload.label}</span><span>{src.label}</span>
    <span>{link.label}</span><span>{link.title}</span>
    <For each={DATE_PRESETS}>{(preset) => <span>{preset.label}</span>}</For>
    <For each={PAGE_PROP_SPECS}>{(spec) => <span title={spec.hint}>{spec.label}</span>}</For>
    <For each={SEARCH_SYNTAX}>{(syntax) => <span>{syntax.description}</span>}</For>
  </>, root);
  try {
    expect(root.textContent).toContain("Upload an assetSrcLinkMarkdown linkTodayLast 7 days");
    locale.set(true);
    expect(root.textContent).toContain("«Upload an asset»«Src»«Link»«Markdown link»«Today»«Last 7 days»");
    expect(root.querySelector("[title]")?.getAttribute("title")).toBe("«Other names this page answers to in [[links]] (comma-separated)»");
    expect(filterCommands("upload")).toEqual(matching);
    expect(advancedBlockInsertion(src, "md")).toEqual({ insert: "```\n\n```", caret: 3 });
    expect(COMMANDS.find((command) => command.taskMarker === "TODO")?.label).toBe("TODO");
  } finally { dispose(); }
});

it("an open PDF export dialog updates its labels and emphasis without changing the page name", () => {
  setPdfExportPage("Graph page 中文");
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <PdfExportDialog />, root);
  try {
    expect(root.querySelector(".pdf-export-note")?.textContent).toBe("Opens your system print dialog — choose Save as PDF (or a printer). The document always prints on a light background, whatever your theme.");
    expect(root.querySelector(".pdf-export-note b")?.textContent).toBe("Save as PDF");
    expect(root.querySelector("[title]")?.getAttribute("title")).toBe("print every block, even folded ones");
    locale.set(true);
    expect(root.textContent).toContain("«Collapsed blocks»");
    expect(root.textContent).toContain("«Expand all»");
    expect(root.querySelector("[title]")?.getAttribute("title")).toBe("«print every block, even folded ones»");
    expect(root.querySelector(".export-count")?.textContent).toBe("Graph page 中文");
    expect(root.querySelector(".pdf-export-note b")?.textContent).toBe("Save as PDF");
  } finally { dispose(); }
});

it("a column resize control translates its accessible instructions while retaining the authored field label", () => {
  setDoc({
    byId: { table: { id: "table", raw: "Table", collapsed: false, parent: null, page: "Sheet", children: [] } },
    pages: [{ name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["table"], format: "md", readOnly: false, guide: false }],
    feed: ["Sheet"], loaded: true,
  });
  const root = document.createElement("div");
  const dispose = render(() => {
    const resize = createTableColumnResize("table");
    return resize.handle("author-key", "作者");
  }, root);
  try {
    expect(root.querySelector("button")?.getAttribute("aria-label")).toBe("Resize 作者 column");
    locale.set(true);
    expect(root.querySelector("button")?.getAttribute("aria-label")).toBe("«Resize 作者 column»");
    expect(root.querySelector("button")?.title).toBe("«Drag to resize; double-click to reset»");
    expect(root.querySelector("button")?.getAttribute("data-sheet-resize-handle")).toBe("author-key");
  } finally { dispose(); }
});
