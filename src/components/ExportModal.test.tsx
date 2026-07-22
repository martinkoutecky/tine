import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ExportModal } from "./ExportModal";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore, setDoc, type Node as StoreNode } from "../store";
import { closeExportModal, openExportModal } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";

beforeAll(async () => {
  await initParser();
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
});
