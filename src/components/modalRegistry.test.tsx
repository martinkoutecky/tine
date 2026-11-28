// The open-modal registry's lifetime. A registration that outlived its modal
// would leave every outline key refused, so each registered modal releases it
// on every real close route, an error boundary's disposal and a closed window,
// and keeps it on a press that only peels a layer inside the modal.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { clearDelegatedEvents, DelegatedEvents, delegateEvents, render } from "solid-js/web";
import { createSignal, onMount, Show, type JSX } from "solid-js";
import { Settings } from "./Settings";
import { CssEditor } from "./CssEditor";
import { Lightbox } from "./Toasts";
import { AudioOverlay } from "./AudioOverlay";
import { QuickSwitcher } from "./QuickSwitcher";
import { WelcomeLayer } from "./Welcome";
import { GraphNamePrompt } from "./GraphNamePrompt";
import { LinkSchemePrompt } from "./LinkSchemePrompt";
import { CommentNamePrompt } from "./CommentNamePrompt";
import { RenameDialogFrame } from "./RenameDialogFrame";
import { DeepLinkGraphChoice, chooseLinkGraph } from "./DeepLinkGraphChoice";
import { CloneGraphDialog } from "./CloneGraphDialog";
import { UnsavedRecovery } from "./UnsavedRecovery";
import { ReferenceExportChooser } from "./ReferenceExportChooser";
import { QuickShareAll } from "./QuickShareAll";
import { QuickShareDialog } from "./QuickShareDialog";
import { ShareDialog } from "./ShareDialog";
import { QueryExportDialog } from "./QueryExportDialog";
import { PdfExportDialog } from "./PdfExportDialog";
import { ExportModal } from "./ExportModal";
import { QueryWorkspace, type QueryWorkspaceDependencies } from "./QueryWorkspace";
import { FailureBoundary } from "./FailureBoundary";
import { WindowOverlays } from "./WindowOverlays";
import * as graph from "../graph";
import { backend } from "../backend";
import {
  closeExportModal, closePdfExport, closeQueryExport, closeSettings, closeSwitcher, openExportModal, openPdfExport,
  openQueryExport, openSettings, openSwitcher, queryExportRequest, setAudioPlayer, setLightbox, settingsOpen,
} from "../ui";
import { cancelCssEditor, cssEditorSession, openCssEditor } from "../cssEditor";
import { askGraphName, setGraphNameRequest } from "../graphNamePrompt";
import { askLinkScheme } from "../linkSchemePrompt";
import { setCommentNameRequest } from "../commentNamePrompt";
import { closeUnsavedRecovery, openUnsavedRecovery } from "../unsavedRecovery";
import { closeQuickShare, closeShareDialog, openQuickShare, openShareDialog, quickShareRequest, shareTarget } from "../shareState";
import { clearTransientLayersForTest, modalOpen, registerModal, useModal } from "../transientLayers";
import { installKeybindings } from "../keybindings";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { pageRoots, resetStore, selectBlock } from "../document";
import { endEdit } from "../editorController";
import { MAIN_WINDOW_ID, WindowContext, registerWindow, setActiveWindowId } from "../windowRealm";
import type { PaneRouter, QueryRoute } from "../router";
import type { QueryPublicationRequest } from "../types";

beforeAll(() => initParser());

const cleanups: (() => void)[] = [];
const settle = async () => { for (let i = 0; i < 4; i++) await new Promise((resolve) => setTimeout(resolve, 0)); };

afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) cleanup();
  closeSettings(); closeSwitcher(); setLightbox(null); setAudioPlayer(null);
  if (cssEditorSession()) await cancelCssEditor();
  setGraphNameRequest(null); setCommentNameRequest(null); closeUnsavedRecovery();
  closeQuickShare(); closeShareDialog(); closeQueryExport(); closePdfExport(); closeExportModal();
  endEdit("page-navigation");
  resetStore();
  setActiveWindowId(MAIN_WINDOW_ID);
  clearTransientLayersForTest();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

function mount(view: () => JSX.Element, host: HTMLElement = document.body): { root: HTMLElement; dispose: () => void } {
  const root = host.ownerDocument.createElement("div");
  host.append(root);
  let disposed = false;
  const unmount = render(view, root);
  const dispose = () => { if (!disposed) { disposed = true; unmount(); root.remove(); } };
  cleanups.push(dispose);
  return { root, dispose };
}

function escape(target: Element = document.activeElement instanceof HTMLElement ? document.activeElement : document.body) {
  target.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
}
function clickText(label: string) {
  const button = [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.trim() === label);
  if (!button) throw new Error(`missing button ${label}`);
  button.click();
}
function click(selector: string) {
  const element = document.querySelector<HTMLElement>(selector);
  if (!element) throw new Error(`missing ${selector}`);
  element.click();
}

/** Test 12's key: with no modal, Backspace on a selected block deletes it. */
function outlineKeysWork() {
  loadSingle({ name: "K", kind: "page", title: "K", id: "pages/K.md", rev: "r1", format: "md", pre_block: null, blocks: [
    { id: "k1", raw: "one", children: [], collapsed: false }, { id: "k2", raw: "two", children: [], collapsed: false },
  ] });
  selectBlock("k1");
  (document.activeElement as HTMLElement | null)?.blur?.();
  document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Backspace", bubbles: true, cancelable: true }));
  expect([...pageRoots("K")]).toEqual(["k2"]);
}

const REQUEST: QueryPublicationRequest = { argument: "@page and name = 'Project'", dialect: "macro_tql", properties: [],
  currentPage: "Project", hostBlockId: null, name: "Project", includeQueryText: false };

function stubMedia() {
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
}

function queryRouter(route: QueryRoute): PaneRouter {
  return { route: vi.fn(() => route), updateActiveQuery: vi.fn(), replaceActiveRoute: vi.fn(), openPage: vi.fn(),
    openInNewTab: vi.fn(), openPageTarget: vi.fn(), openPageAtBlock: vi.fn() } as unknown as PaneRouter;
}
function queryDeps(): QueryWorkspaceDependencies {
  const empty = { hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false };
  return {
    serializeDsl: vi.fn(async (source: string) => ({ query: `{{query ${source}}}`, properties: [] })),
    resolvePage: vi.fn(async (name: string) => ({ kind: "absent" as const, id: `pages/${name}.md` })),
    createPage: vi.fn(async () => "saved-rev"),
    runGraphSearch: vi.fn(async () => empty),
    parseQuery: vi.fn((source: string) => backend().parseQuery(source, "macro_query")),
    queryRun: vi.fn(async () => ({ anchor: "block" as const, groups: [], diagnostics: [], report: { ran: [], ignored: [], supported: true }, total: 0, exceeded: false })),
    queryExplainEmpty: vi.fn(async () => ({ rows: [], diagnostics: [], report: { ran: [], ignored: [], supported: true } })),
  } as QueryWorkspaceDependencies;
}
function QueryAdvanced(): JSX.Element {
  const route: QueryRoute = { kind: "query", id: "modal-registry", sourceKind: "search", source: "", presentation: "search" };
  return <QueryWorkspace route={route} router={queryRouter(route)} deps={queryDeps()} />;
}

type Case = {
  name: string;
  mount: () => JSX.Element;
  open: () => unknown;
  routes: Record<string, () => unknown>;
};

const [renameOpen, setRenameOpen] = createSignal(false);
const [welcomeOpen, setWelcomeOpen] = createSignal(false);
const [cloneOpen, setCloneOpen] = createSignal(false);
const [chooserOpen, setChooserOpen] = createSignal(false);

const cases: Case[] = [
  { name: "Settings", mount: () => <Settings />, open: () => { stubMedia(); openSettings(); },
    routes: { Escape: () => escape(), "the close button": () => click(".settings-close"), "a backdrop click": () => click(".modal-overlay"), "the state reset": closeSettings } },
  { name: "CSS Editor", mount: () => <CssEditor />, open: () => openCssEditor(true),
    routes: { Escape: () => escape(), "Cancel": () => clickText("Cancel"),
      "Save and close": () => click(".settings-btn-primary"), "a backdrop click": () => click(".modal-overlay"), "the state reset": cancelCssEditor } },
  { name: "the image viewer", mount: () => <Lightbox />, open: () => setLightbox("blob:image"),
    routes: { Escape: () => escape(), "the close button": () => click(".lightbox-close"), "a backdrop click": () => click(".lightbox-overlay"), "the state reset": () => setLightbox(null) } },
  { name: "expanded audio", mount: () => <AudioOverlay />, open: () => setAudioPlayer({ url: "blob:audio", name: "clip.mp3" }),
    routes: { Escape: () => escape(), "the close button": () => click(".audio-close"), "a backdrop click": () => click(".audio-overlay"), "the state reset": () => setAudioPlayer(null) } },
  { name: "the switcher", mount: () => <QuickSwitcher />, open: () => openSwitcher(),
    routes: { Escape: () => escape(), "a backdrop click": () => click(".switcher-overlay"), "the state reset": closeSwitcher } },
  { name: "the command palette", mount: () => <QuickSwitcher />, open: () => openSwitcher({ mode: "pages" }),
    routes: { Escape: () => escape(), "the state reset": closeSwitcher } },
  { name: "optional Welcome", mount: () => <WelcomeLayer mandatory={false} optionalOpen={welcomeOpen()} onClose={() => setWelcomeOpen(false)} />,
    open: () => setWelcomeOpen(true),
    routes: { Escape: () => escape(), "the close button": () => click(".welcome-close"), "the state reset": () => setWelcomeOpen(false) } },
  { name: "the graph-name prompt", mount: () => <GraphNamePrompt />, open: () => { void askGraphName("G", async () => null); },
    routes: { Escape: () => escape(), "Cancel": () => click(".graph-name-prompt .export-btn-secondary"), "a backdrop click": () => click(".modal-overlay"), "the request reset": () => setGraphNameRequest(null) } },
  { name: "the link-scheme prompt", mount: () => <LinkSchemePrompt />, open: () => { void askLinkScheme("zotero", "zotero://x", () => true); },
    routes: { Escape: () => escape(), "Cancel": () => click(".link-scheme-prompt .export-btn-secondary"), "a backdrop click": () => click(".modal-overlay"),
      "Open (a successful action)": () => click(".link-scheme-prompt .export-btn-primary") } },
  { name: "the comment-name prompt", mount: () => <CommentNamePrompt />, open: () => setCommentNameRequest({ finish: () => setCommentNameRequest(null) }),
    routes: { Escape: () => escape(), "Skip": () => click(".comment-name-prompt .export-btn-secondary"), "a backdrop click": () => click(".modal-overlay"), "the request reset": () => setCommentNameRequest(null) } },
  { name: "the rename frame", mount: () => <Show when={renameOpen()}><RenameDialogFrame id="rename-test" label="Rename" dismiss={() => setRenameOpen(false)}><button data-choice onClick={() => setRenameOpen(false)}>Keep</button></RenameDialogFrame></Show>,
    open: () => setRenameOpen(true),
    routes: { Escape: () => escape(), "its choice": () => click("button[data-choice]"), "the request reset": () => setRenameOpen(false) } },
  { name: "the deep-link graph choice", mount: () => <DeepLinkGraphChoice />, open: () => { void chooseLinkGraph([{ root: "/a" }, { root: "/b" }]); },
    routes: { Escape: () => escape(), "a choice": () => click(".tine-link-roots button"), "Cancel": () => click(".tine-link-choice > button") } },
  { name: "Clone", mount: () => <Show when={cloneOpen()}><CloneGraphDialog onClose={() => setCloneOpen(false)} /></Show>, open: () => setCloneOpen(true),
    routes: { Escape: () => escape(), "Cancel": () => click(".export-modal .export-btn-secondary"), "a backdrop click": () => click(".modal-overlay"), "the state reset": () => setCloneOpen(false) } },
  { name: "unsaved recovery", mount: () => <UnsavedRecovery />, open: openUnsavedRecovery,
    routes: { Escape: () => escape(), "Keep working": () => clickText("Keep working"), "the state reset": closeUnsavedRecovery } },
  { name: "the reference export chooser", mount: () => <Show when={chooserOpen()}><ReferenceExportChooser subject="S" groups={[]} onClose={() => setChooserOpen(false)} /></Show>,
    open: () => setChooserOpen(true),
    routes: { Escape: () => escape(), "a backdrop click": () => click(".modal-overlay"), "the state reset": () => setChooserOpen(false) } },
  { name: "Quick Share All", mount: () => <QuickShareAll />, open: () => click(".quick-share-all"),
    routes: { Escape: () => escape(), "a backdrop click": () => click(".modal-overlay") } },
  { name: "Quick Share", mount: () => <QuickShareDialog request={quickShareRequest} />, open: () => openQuickShare(REQUEST),
    routes: { Escape: () => escape(), "a backdrop click": () => click(".modal-overlay"), "the request reset": closeQuickShare } },
  { name: "Share", mount: () => <ShareDialog target={shareTarget} />, open: () => openShareDialog({ page: "P" }),
    routes: { Escape: () => escape(), "a backdrop click": () => click(".modal-overlay"), "the request reset": closeShareDialog } },
  { name: "query export", mount: () => <QueryExportDialog request={queryExportRequest} />, open: () => openQueryExport(REQUEST),
    routes: { Escape: () => escape(), "a backdrop click": () => click(".modal-overlay"), "the request reset": closeQueryExport } },
  { name: "PDF export", mount: () => <PdfExportDialog />, open: () => openPdfExport("P"),
    routes: { Escape: () => escape(), "a backdrop click": () => click(".modal-overlay"), "the request reset": closePdfExport } },
  { name: "Copy / Export", mount: () => <ExportModal />, open: () => openExportModal(["k-export"]),
    routes: { Escape: () => escape(), "a backdrop click": () => click(".modal-overlay"), "the request reset": closeExportModal } },
  { name: "the query Advanced form", mount: () => <QueryAdvanced />, open: () => click(".query-advanced-toggle"),
    routes: { Escape: () => escape(document.querySelector(".query-advanced-modal") ?? document.body) } },
];

describe("every registered modal releases its registration on each real close route", () => {
  const rows = cases.flatMap((c) => Object.keys(c.routes).map((route) => [c.name, route, c] as const));
  it.each(rows)("%s: %s", async (_name, route, c) => {
    loadSingle({ name: "P", kind: "page", title: "P", id: "pages/P.md", rev: "r1", format: "md", pre_block: null,
      blocks: [{ id: "k-export", raw: "export me", children: [], collapsed: false }] });
    cleanups.push(installKeybindings());
    mount(c.mount);
    await settle();
    expect(modalOpen()).toBe(false);
    await c.open();
    await settle();
    expect(modalOpen()).toBe(true);
    await c.routes[route]();
    await vi.waitFor(() => expect(modalOpen()).toBe(false));
    resetStore();
    outlineKeysWork();
  });
});

describe("a press that peels a layer inside the modal keeps its registration", () => {
  it("the first Escape in Settings with a query", async () => {
    cleanups.push(installKeybindings());
    stubMedia();
    mount(() => <Settings />);
    openSettings();
    await settle();
    const box = document.querySelector<HTMLInputElement>(".settings-search-input")!;
    box.focus(); box.value = "theme"; box.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await settle();
    escape(box);
    await settle();
    expect(settingsOpen()).toBe(true);
    expect(modalOpen()).toBe(true);
    escape(box);
    await vi.waitFor(() => expect(modalOpen()).toBe(false));
  });

  it("the switcher with its search syntax open", async () => {
    cleanups.push(installKeybindings());
    mount(() => <QuickSwitcher />);
    openSwitcher();
    await settle();
    click(".switcher-syntax-toggle");
    await settle();
    expect(document.querySelector(".switcher-syntax")).not.toBeNull();
    escape(document.body);
    await settle();
    expect(document.querySelector(".switcher-syntax")).toBeNull();
    expect(modalOpen()).toBe(true);
    escape(document.body);
    await vi.waitFor(() => expect(modalOpen()).toBe(false));
  });

  it("the image viewer with its menu open", async () => {
    cleanups.push(installKeybindings());
    mount(() => <Lightbox />);
    setLightbox("blob:image");
    await settle();
    document.querySelector(".lightbox-img")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 5, clientY: 5 }));
    await settle();
    expect(document.querySelector(".lightbox-menu")).not.toBeNull();
    escape(document.body);
    await settle();
    expect(document.querySelector(".lightbox-menu")).toBeNull();
    expect(modalOpen()).toBe(true);
    escape(document.body);
    await vi.waitFor(() => expect(modalOpen()).toBe(false));
  });

  it("a busy Clone declining dismissal", async () => {
    cleanups.push(installKeybindings());
    vi.spyOn(graph, "chooseNewGraphParent").mockReturnValue(new Promise(() => {}));
    const [open, setOpen] = createSignal(true);
    mount(() => <Show when={open()}><CloneGraphDialog onClose={() => setOpen(false)} /></Show>);
    await settle();
    const url = document.querySelector<HTMLInputElement>(".export-modal input[type=url]")!;
    url.value = "https://example.org/repo.git";
    url.dispatchEvent(new InputEvent("input", { bubbles: true }));
    document.querySelector("form.export-modal")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    await settle();
    expect(document.querySelector("[role=status]")).not.toBeNull();
    escape(document.body);
    click(".modal-overlay");
    await settle();
    expect(open()).toBe(true);
    expect(modalOpen()).toBe(true);
  });
});

describe("registration lifetime", () => {
  it("a modal whose later onMount throws inside a FailureBoundary releases its registration when the boundary disposes it", async () => {
    let throwing = true;
    function Throws(): JSX.Element {
      useModal();
      onMount(() => { if (throwing) throw new Error("mount failed"); });
      return <div class="modal-overlay" />;
    }
    vi.spyOn(console, "error").mockImplementation(() => {});
    mount(() => <FailureBoundary region="Test"><Throws /></FailureBoundary>);
    await settle();
    expect(document.querySelector(".modal-overlay")).toBeNull();
    expect(modalOpen()).toBe(false);
    throwing = false;
  });

  it("a throw before the registration effect runs acquires nothing", async () => {
    function ThrowsFirst(): JSX.Element {
      throw new Error("constructor failed");
    }
    vi.spyOn(console, "error").mockImplementation(() => {});
    mount(() => <FailureBoundary region="Test"><ThrowsFirst /></FailureBoundary>);
    await settle();
    expect(modalOpen()).toBe(false);
  });

  it("a disposer called twice leaves the count at zero, and another modal's registration survives it", () => {
    const first = registerModal();
    const second = registerModal();
    first();
    first();
    expect(modalOpen()).toBe(true);
    second();
    expect(modalOpen()).toBe(false);
    second();
    expect(modalOpen()).toBe(false);
  });

  it("Clone opened from Welcome: closing Clone leaves Welcome registered; closing Welcome releases the last", async () => {
    cleanups.push(installKeybindings());
    const [open, setOpen] = createSignal(true);
    mount(() => <WelcomeLayer mandatory={false} optionalOpen={open()} onClose={() => setOpen(false)} />);
    await settle();
    const clone = [...document.querySelectorAll<HTMLButtonElement>(".welcome-actions button, .welcome-card button")]
      .find((button) => /clone/i.test(button.textContent ?? ""));
    expect(clone).toBeTruthy();
    clone!.click();
    await settle();
    expect(document.querySelectorAll(".modal-overlay")).toHaveLength(1);
    click(".modal-overlay .export-btn-secondary");
    await settle();
    expect(document.querySelector(".modal-overlay")).toBeNull();
    expect(modalOpen()).toBe(true);
    setOpen(false);
    await settle();
    expect(modalOpen()).toBe(false);
  });

  it("Settings open in a workspace window that closes shows in main and stays registered; dismissing it releases it", async () => {
    stubMedia();
    cleanups.push(installKeybindings());
    const frame = document.createElement("iframe");
    document.body.append(frame);
    const unregister = registerWindow("ws-settings", frame.contentWindow!);
    mount(() => <WindowContext.Provider value={MAIN_WINDOW_ID}><WindowOverlays windowId={MAIN_WINDOW_ID} /></WindowContext.Provider>);
    const workspace = mount(() => <WindowContext.Provider value="ws-settings"><WindowOverlays windowId="ws-settings" /></WindowContext.Provider>, frame.contentDocument!.body);
    setActiveWindowId("ws-settings");
    openSettings();
    await vi.waitFor(() => expect(frame.contentDocument!.querySelector(".settings-modal")).not.toBeNull());
    expect(modalOpen()).toBe(true);
    workspace.dispose();
    unregister();
    setActiveWindowId(MAIN_WINDOW_ID);
    await vi.waitFor(() => expect(document.querySelector(".settings-modal")).not.toBeNull());
    expect(settingsOpen()).toBe(true);
    expect(modalOpen()).toBe(true);
    closeSettings();
    await settle();
    expect(modalOpen()).toBe(false);
    outlineKeysWork();
  });

  it("the query Advanced form open in a workspace window that closes releases its registration", async () => {
    const frame = document.createElement("iframe");
    document.body.append(frame);
    const unregister = registerWindow("ws-query", frame.contentWindow!);
    delegateEvents([...DelegatedEvents], frame.contentDocument!);
    cleanups.push(() => clearDelegatedEvents(frame.contentDocument!));
    const workspace = mount(() => <WindowContext.Provider value="ws-query"><QueryAdvanced /></WindowContext.Provider>, frame.contentDocument!.body);
    await settle();
    workspace.root.querySelector<HTMLButtonElement>(".query-advanced-toggle")!.click();
    await settle();
    expect(modalOpen()).toBe(true);
    workspace.dispose();
    unregister();
    expect(modalOpen()).toBe(false);
  });
});
