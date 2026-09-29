import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ExportModal } from "./ExportModal";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { type Node as StoreNode } from "../document/model";
import { setDoc } from "../document/model";
import { closeExportModal, openExportModal } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";
import { setToasts, toasts } from "../toasts";

beforeAll(async () => {
  await initParser();
});

describe("ExportModal content choice (GH #352)", () => {
  const node = (id: string, raw: string, parent: string | null, page = "P", children: string[] = []): StoreNode => ({
    id, raw, collapsed: false, parent, page, children,
  });

  beforeEach(() => {
    localStorage.clear();
    setDoc({
      byId: {
        root: node("root", "Root [[Page]] **bold**\nproperty:: hidden", null, "P", ["child"]),
        child: node("child", "Child ==highlight==", "root", "P"),
        org: node("org", "*bold* /it/ text", null, "O"),
      },
      pages: [
        { name: "P", kind: "page", title: "P", preBlock: null, roots: ["root"], format: "md", readOnly: false, guide: false },
        { name: "O", kind: "page", title: "O", preBlock: null, roots: ["org"], format: "org", readOnly: false, guide: false },
      ],
      feed: ["P", "O"],
      loaded: true,
    });
  });

  afterEach(() => {
    closeExportModal();
    clearTransientLayersForTest();
    resetStore();
    document.body.innerHTML = "";
  });

  it("offers an explicit preserve-Markdown choice next to the cleaned plain-text one", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <ExportModal />, root);

    openExportModal(["root"]);
    await Promise.resolve();

    const buttons = () => [...document.querySelectorAll<HTMLButtonElement>("button")];
    const byText = (label: string) => buttons().find((button) => button.textContent?.trim() === label);
    const preview = () => document.querySelector<HTMLTextAreaElement>(".export-preview")!.value;

    // The GH #352 gap was discoverability: the choice existed but only under
    // internal mode names. Both choices must name the outcome the user gets.
    expect(byText("Plain text")).toBeDefined();
    const preserve = byText("Markdown");
    expect(preserve).toBeDefined();

    // Default remains the cleaned/plain-text choice: markers flattened away.
    expect(preview()).toContain("bold");
    expect(preview()).not.toContain("**");

    // The preserve choice keeps the representative source syntax byte-exact,
    // on the block entry point here and (shared modal) the page entry point.
    preserve!.click();
    await Promise.resolve();
    expect(preview()).toContain("**bold**");
    expect(preview()).toContain("[[Page]]");
    expect(preview()).toContain("property:: hidden");
    expect(preview()).toContain("==highlight==");

    dispose();
  });

  it("names the preserved syntax after the page's own format (Org pages)", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <ExportModal />, root);

    openExportModal(["org"]);
    await Promise.resolve();

    const buttons = () => [...document.querySelectorAll<HTMLButtonElement>("button")];
    const preserve = buttons().find((button) => button.textContent?.trim() === "Org");
    expect(preserve).toBeDefined();

    preserve!.click();
    await Promise.resolve();
    const preview = document.querySelector<HTMLTextAreaElement>(".export-preview")!.value;
    expect(preview).toContain("*bold* /it/ text");

    dispose();
  });
});

describe("ExportModal formats", () => {
  const node = (id: string, raw: string, parent: string | null, children: string[]): StoreNode => ({
    id, raw, collapsed: false, parent, page: "P", children,
  });

  beforeEach(() => {
    localStorage.clear();
    setDoc({
      byId: {
        root: node("root", "Root [[Page]] **bold**\nproperty:: hidden", null, ["child"]),
        child: node("child", "Child", "root", []),
      },
      pages: [{ name: "P", kind: "page", title: "P", preBlock: null, roots: ["root"], format: "md", readOnly: false, guide: false }],
      feed: ["P"],
      loaded: true,
    });
  });

  afterEach(() => {
    closeExportModal();
    clearTransientLayersForTest();
    resetStore();
    document.body.innerHTML = "";
    vi.restoreAllMocks();
    setToasts([]);
  });

  it("shows Text/OPML/HTML without PNG, scopes options, and copies the selected payload", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <ExportModal />, root);
    const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);

    openExportModal(["root"]);
    await Promise.resolve();

    const buttons = () => [...document.querySelectorAll<HTMLButtonElement>("button")];
    const byText = (label: string) => buttons().find((button) => button.textContent?.trim() === label);
    expect(byText("Text")).toBeDefined();
    expect(byText("OPML")).toBeDefined();
    expect(byText("HTML")).toBeDefined();
    expect(byText("PNG")).toBeUndefined();
    expect(document.body.textContent).toContain("Indent");
    expect(document.body.textContent).toContain("Remove properties");
    expect(document.body.textContent).toContain("Newline after block");

    byText("OPML")!.click();
    await Promise.resolve();
    expect(document.body.textContent).not.toContain("Indent");
    expect(document.body.textContent).not.toContain("Remove properties");
    expect(document.body.textContent).not.toContain("Newline after block");
    expect(document.body.textContent).toContain("[[links]] → text");
    expect(document.body.textContent).toContain("Remove emphasis");
    expect(document.body.textContent).toContain("Remove #tags");
    expect(document.body.textContent).toContain("Level ≤");

    const preview = document.querySelector<HTMLTextAreaElement>(".export-preview")!.value;
    expect(preview).toContain("<opml");
    expect(preview).not.toContain("property");
    byText("Copy")!.click();
    expect(writeText).toHaveBeenCalledWith(preview);
    await Promise.resolve();

    openExportModal(["root"]);
    await Promise.resolve();
    byText("HTML")!.click();
    await Promise.resolve();
    expect(document.body.textContent).not.toContain("Indent");
    expect(document.body.textContent).not.toContain("Remove properties");
    expect(document.body.textContent).not.toContain("Newline after block");
    const htmlPreview = document.querySelector<HTMLTextAreaElement>(".export-preview")!.value;
    expect(htmlPreview).toContain("<ul>");
    expect(htmlPreview).toContain("<strong>bold</strong>");
    expect(htmlPreview).not.toContain("property");
    byText("Copy")!.click();
    expect(writeText).toHaveBeenLastCalledWith(htmlPreview);
    dispose();
  });

  it("waits for the clipboard before reporting Copy success", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <ExportModal />, root);
    let reject!: (error: Error) => void;
    vi.spyOn(backend(), "writeText").mockReturnValue(new Promise<void>((_, fail) => { reject = fail; }));
    openExportModal(["root"]);
    await Promise.resolve();
    const byText = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.trim() === label);
    byText("OPML")!.click();
    await Promise.resolve();
    byText("Copy")!.click();
    expect(toasts().some((toast) => toast.message === "Copied to clipboard")).toBe(false);
    reject(new Error("clipboard denied"));
    await vi.waitFor(() => expect(toasts().some((toast) => toast.kind === "error")).toBe(true));
    dispose();
  });
});
