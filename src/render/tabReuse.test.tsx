import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { refreshPageIndex, resetPageIndex } from "../pageIndex";
import { paneRouter, resetPaneLayoutToSingle } from "../panes";
import { setNavReuseTabs } from "../navSettings";
import { TabBar } from "../components/TabBar";
import { InlineText } from "./inline";
import { createPaneRouter, type Route } from "../router";

beforeEach(async () => {
  setNavReuseTabs(true);
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [{
    key: "target", name: "Target", is_journal: false, day: null,
    target: { kind: "existing", id: "pages/Target.md", others: ["pages/stray/Target.org"] },
  }] });
  await refreshPageIndex();
});

afterEach(() => {
  resetPageIndex();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

function setup(target: Route) {
  const routes: Route[] = [target, { kind: "page", name: "Source", pageKind: "page" }];
  resetPaneLayoutToSingle({ tabs: routes
    .map((r) => ({ history: [r], pos: 0, pinned: false })), activeIndex: 1 });
  const router = paneRouter("main");
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <><TabBar router={router} /><InlineText text="[[Target]]" /></>, host);
  return { router, host, dispose };
}

it.each(["page", "journal"] as const)("plain %s names reuse the canonical physical tab", async (kind) => {
  if (kind === "journal") {
    vi.mocked(backend().pageInventory).mockResolvedValue({ rev: "2", entries: [{
      key: "target", name: "Target", is_journal: true, day: 20261016,
      target: { kind: "existing", id: "journals/Target.md", others: [] },
    }] });
    await refreshPageIndex();
  }
  const { router, host, dispose } = setup({ kind: "page", name: "Target", pageKind: kind, path: `${kind === "page" ? "pages" : "journals"}/Target.md` });
  try {
    if (kind === "page") host.querySelector<HTMLElement>(".page-ref")!.click();
    else router.openPage("Target", "journal");
    expect(host.querySelector(".tab.active")?.textContent).toContain("Target");
    expect(router.tabs().map((t) => router.tabRoute(t))).toMatchObject([{ name: "Target" }, { name: "Source" }]);
  } finally { dispose(); }
});

it("physical search/history navigation reuses a name tab", () => {
  const { router, host, dispose } = setup({ kind: "page", name: "Target", pageKind: "page" });
  try {
    router.openFile("pages/Target.md", "Target", "page");
    expect(host.querySelector(".tab.active")?.textContent).toContain("Target");
    expect(router.tabRoute(router.tabs()[1])).toMatchObject({ name: "Source" });
  } finally { dispose(); }
});

it.each([
  { path: "pages/stray/Target.org" },
  { path: "pages/Target.md", block: "zoomed" },
])("does not reuse a different physical owner or zoom: %j", (extra) => {
  const { router, host, dispose } = setup({ kind: "page", name: "Target", pageKind: "page", ...extra });
  try {
    host.querySelector<HTMLElement>(".page-ref")!.click();
    expect(router.activeId()).toBe(router.tabs()[1].id);
  } finally { dispose(); }
});

it("OFF and explicit in-place keep navigation in the source tab", () => {
  const { router, host, dispose } = setup({ kind: "page", name: "Target", pageKind: "page", path: "pages/Target.md" });
  try {
    setNavReuseTabs(false);
    host.querySelector<HTMLElement>(".page-ref")!.click();
    expect(router.activeId()).toBe(router.tabs()[1].id);
    setNavReuseTabs(true);
    router.openPage("Source", "page", { inPlace: true });
    router.openPage("Target", "page", { inPlace: true });
    expect(router.activeId()).toBe(router.tabs()[1].id);
  } finally { dispose(); }
});

it("explicit new-tab navigation still duplicates the destination", () => {
  const { router, dispose } = setup({ kind: "page", name: "Target", pageKind: "page", path: "pages/Target.md" });
  try {
    router.openPageInNewTab("Target");
    expect(router.tabs()).toHaveLength(3);
    expect(router.tabRoute(router.tabs()[1])).toMatchObject({ name: "Source" });
    expect(router.activeId()).toBe(router.tabs()[1].id);
  } finally { dispose(); }
});

it("reuses within the destination pane without activating a matching tab in another pane", () => {
  const { router: main, dispose } = setup({ kind: "page", name: "Target", pageKind: "page", path: "pages/Target.md" });
  const other = createPaneRouter("reuse-other");
  try {
    other.openFile("pages/Target.md", "Target", "page");
    const targetId = other.activeId();
    other.openInNewTab({ kind: "page", name: "Other source", pageKind: "page" }, true);
    other.openPage("Target");
    expect(other.activeId()).toBe(targetId);
    expect(other.tabRoute(other.tabs()[1])).toMatchObject({ name: "Other source" });
    expect(main.route()).toMatchObject({ name: "Source" });
  } finally { dispose(); }
});

it("does not guess a physical owner before the inventory is available", () => {
  const { router, host, dispose } = setup({ kind: "page", name: "Target", pageKind: "page", path: "pages/Target.md" });
  try {
    resetPageIndex();
    host.querySelector<HTMLElement>(".page-ref")!.click();
    expect(router.activeId()).toBe(router.tabs()[1].id);
  } finally { dispose(); }
});
