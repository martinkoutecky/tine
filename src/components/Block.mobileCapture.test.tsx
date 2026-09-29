// GH #493 research fixture: the stale-editor-token branch of mobile photo
// capture. The reporter's toast is exactly reportStaleAsset() ("The asset was
// saved, but it was not inserted because the graph or block changed."), so the
// Android failure is NOT in the native capture bridge or the asset copy — the
// pick and the durable save into assets/ both succeed, and only the Markdown
// insertion is refused.
//
// The Android-specific trigger is the external picker/camera activity: while
// the native chooser is up, the WebView is suspended/blurred. When that blur
// reaches the block editor WITHOUT the document having lost window focus
// (Android pauses the activity asynchronously; the focus/blur ordering is not
// the desktop WebKitGTK one), Block.tsx's onBlur takes the "real exit" branch
// and ends edit mode. The editor token captured when the camera button was
// tapped is then stale by the time capturePhoto/importNativeCapture resolve.
//
// This harness reproduces that branch at the exact JS seam the Android WebView
// drives: it dispatches the real mobile editor command, keeps both native
// awaits pending (as the picker activity does), performs the lifecycle change
// mid-flight, and then proves "asset saved but Markdown not inserted" plus the
// literal toast. The window-blur-protected case below is the desktop control:
// the same blur with document.hasFocus() false keeps edit mode, which is why
// the GTK file dialog never trips this on desktop.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { startEditing } from "../editorController";
import { dispatchFocusedEditorCommand } from "../editorCommandBridge";
import { setGraphMeta } from "../graphSession";
import { setToasts, toasts } from "../toasts";
import { resetSaveState } from "../document/save/engine";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";
import { isRecordingAudio, setRecordingAudio } from "../mediaCapture";

const STALE_ASSET_TOAST =
  "The asset was saved, but it was not inserted because the graph or block changed.";

const NATIVE_CACHE_TOKEN = "/data/user/0/page.tine.app/cache/tine_photo_1.jpg";

beforeAll(async () => {
  await initParser();
});

beforeEach(() => {
  setGraphMeta({ root: "/graphs/A" } as never);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  resetSaveState();
  resetStore();
  setToasts([]);
  setGraphMeta(null);
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function blk(id: string, raw: string): BlockDto {
  return { id, raw, collapsed: false, children: [] };
}

function page(name: string, blocks: BlockDto[]): PageDto {
  return { name, kind: "page", title: name, pre_block: null, blocks };
}

function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

async function settle() {
  for (let i = 0; i < 6; i++) await tick();
}

interface CaptureHandles {
  pickedPhoto: (result: { status: "ok"; path: string; ext: string }) => void;
  finishImport: (stored: string) => void;
}

/** Dispatch the real mobile camera command with both native awaits parked, as
 * they are while the Android picker/camera activity covers the app. */
function startCaptureWithPickerUp(handles: CaptureHandles): ReturnType<typeof mount> {
  const pickedPhoto = new Promise<{ status: "ok"; path: string; ext: string }>((resolve) => {
    handles.pickedPhoto = resolve;
  });
  const finishedImport = new Promise<string>((resolve) => {
    handles.finishImport = resolve;
  });
  vi.spyOn(backend(), "capturePhoto").mockImplementation(() => pickedPhoto);
  vi.spyOn(backend(), "importNativeCapture").mockImplementation(() => finishedImport);
  const mounted = mount(() => (
    <For each={pageByName("Assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
  ));
  const textarea = mounted.root.querySelector("textarea") as HTMLTextAreaElement;
  // Focusing registers the focused-editor command bridge the mobile keyboard
  // toolbar dispatches through (Block.tsx onFocus -> registerFocusedEditorBridge).
  textarea.focus();
  expect(dispatchFocusedEditorCommand("editor/capture-photo")).toBe(true);
  return mounted;
}

describe("mobile photo capture editor-token staleness (GH #493)", () => {
  it("preserves the initiating editor across Android's picker blur and inserts the saved asset", async () => {
    loadSingle(page("Assets", [blk("photo-android", "")]));
    const id = pageByName("Assets")!.roots[0];
    startEditing(id, 0);
    const handles = {} as CaptureHandles;

    const { dispose } = startCaptureWithPickerUp(handles);
    try {
      await settle();
      expect(backend().capturePhoto).toHaveBeenCalledOnce();
      // The native picker activity is up: the WebView blur reaches the editor
      // while document.hasFocus() is still true (the Android activity-pause
      // ordering delivers the element blur before the window-focus flip), so
      // onBlur takes the "real exit" branch and ends edit mode. jsdom flips
      // hasFocus() together with the blur event (the desktop ordering), so the
      // Android ordering is pinned explicitly here.
      const hasFocus = vi.spyOn(document, "hasFocus").mockReturnValue(true);
      const textarea = document.querySelector("textarea") as HTMLTextAreaElement;
      textarea.blur();
      hasFocus.mockRestore();

      // The user picked a file; the app resumed and Rust imported it.
      handles.pickedPhoto({ status: "ok", path: NATIVE_CACHE_TOKEN, ext: "jpg" });
      await settle();
      expect(backend().importNativeCapture).toHaveBeenCalledWith(
        NATIVE_CACHE_TOKEN,
        expect.stringMatching(/\.jpg$/),
        expect.any(Number),
        "/graphs/A",
      );
      handles.finishImport("20260916_120000_123-1.jpg");
      await settle();

      // The asset was durably saved and the same initiating editor receives
      // the Markdown despite Android's focus-event ordering.
      expect(backend().importNativeCapture).toHaveBeenCalledOnce();
      expect(doc.byId[id].raw).toBe("![](../assets/20260916_120000_123-1.jpg)");
      expect(toasts().some((toast) => toast.message === STALE_ASSET_TOAST)).toBe(false);
    } finally {
      dispose();
    }
  });

  it("still inserts after the same blur when the window lost focus (desktop dialog control)", async () => {
    loadSingle(page("Assets", [blk("photo-desktop", "")]));
    const id = pageByName("Assets")!.roots[0];
    startEditing(id, 0);
    const handles = {} as CaptureHandles;

    const { dispose } = startCaptureWithPickerUp(handles);
    try {
      await settle();
      // The desktop shape of the same interruption: the OS file dialog takes the
      // window focus, so the editor's blur-preserving branch keeps edit mode.
      const hasFocus = vi.spyOn(document, "hasFocus").mockReturnValue(false);
      const textarea = document.querySelector("textarea") as HTMLTextAreaElement;
      textarea.blur();
      hasFocus.mockRestore();

      handles.pickedPhoto({ status: "ok", path: NATIVE_CACHE_TOKEN, ext: "jpg" });
      await settle();
      handles.finishImport("20260916_120000_123-2.jpg");
      await settle();

      expect(doc.byId[id].raw).toBe("![](../assets/20260916_120000_123-2.jpg)");
      expect(toasts().some((toast) => toast.message === STALE_ASSET_TOAST)).toBe(false);
    } finally {
      dispose();
    }
  });

  it("inserts immediately when the editor stays current through the whole capture", async () => {
    loadSingle(page("Assets", [blk("photo-quiet", "")]));
    const id = pageByName("Assets")!.roots[0];
    startEditing(id, 0);
    const handles = {} as CaptureHandles;

    const { dispose } = startCaptureWithPickerUp(handles);
    try {
      await settle();
      // No lifecycle change at all: pick, import, insert.
      handles.pickedPhoto({ status: "ok", path: NATIVE_CACHE_TOKEN, ext: "jpg" });
      await settle();
      handles.finishImport("20260916_120000_123-3.jpg");
      await settle();

      expect(backend().importNativeCapture).toHaveBeenCalledOnce();
      expect(doc.byId[id].raw).toBe("![](../assets/20260916_120000_123-3.jpg)");
      expect(toasts().some((toast) => toast.message === STALE_ASSET_TOAST)).toBe(false);
    } finally {
      dispose();
    }
  });
});

// Master b3d64add (og-G #29): the recorder is one app-wide state, but the token
// of the editor that started it lived in that Editor's closure. Stopping from
// another editor found no token and returned before importing, so the native
// recording never reached assets/ and nothing was said. It must be stored and
// reported as not inserted.
describe("mobile voice memo stopped from another editor", () => {
  it("imports the recording into assets/ and reports that it was not inserted", async () => {
    loadSingle(page("Memo", [blk("memo-a", "first"), blk("memo-b", "second")]));
    const [a, b] = pageByName("Memo")!.roots;
    vi.spyOn(backend(), "startRecording").mockResolvedValue({ status: "recording" } as never);
    vi.spyOn(backend(), "stopRecording").mockResolvedValue({ status: "ok", path: "/cache/tine_memo_1.m4a", ext: "m4a" } as never);
    const imported = vi.spyOn(backend(), "importNativeCapture").mockResolvedValue("20260929_memo.m4a");
    startEditing(a, 0);
    const mounted = mount(() => <For each={pageByName("Memo")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>);
    try {
      (mounted.root.querySelector("textarea") as HTMLTextAreaElement).focus();
      expect(dispatchFocusedEditorCommand("editor/voice-memo")).toBe(true);
      await settle();
      expect(isRecordingAudio()).toBe(true);
      startEditing(b, 0);
      await settle();
      (mounted.root.querySelector("textarea") as HTMLTextAreaElement).focus();
      expect(dispatchFocusedEditorCommand("editor/voice-memo")).toBe(true);
      await settle();
      expect(isRecordingAudio()).toBe(false);
      expect(imported).toHaveBeenCalledWith("/cache/tine_memo_1.m4a", expect.stringMatching(/\.m4a$/), expect.any(Number), "/graphs/A");
      expect(toasts().some((toast) => toast.message === STALE_ASSET_TOAST)).toBe(true);
      expect(doc.byId[a].raw).toBe("first");
      expect(doc.byId[b].raw).toBe("second");
    } finally {
      setRecordingAudio(false);
      mounted.dispose();
    }
  });
});

// og H1b (manager decision 2026-09-29): a recording finished after a graph
// switch is the only copy of that audio. It must not be lost and must not land
// in the new graph: it is saved into the graph it was started in, named
// explicitly, and the user is told where it went.
describe("mobile voice memo stopped after a graph switch", () => {
  it("saves the recording into the graph it was started in and says where", async () => {
    loadSingle(page("Memo", [blk("memo-a", "first")]));
    vi.spyOn(backend(), "startRecording").mockResolvedValue({ status: "recording" } as never);
    vi.spyOn(backend(), "stopRecording").mockResolvedValue({ status: "ok", path: "/cache/tine_memo_2.m4a", ext: "m4a" } as never);
    const imported = vi.spyOn(backend(), "importNativeCapture").mockResolvedValue("20260929_memo.m4a");
    startEditing(pageByName("Memo")!.roots[0], 0);
    const first = mount(() => <For each={pageByName("Memo")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>);
    let second: ReturnType<typeof mount> | null = null;
    try {
      (first.root.querySelector("textarea") as HTMLTextAreaElement).focus();
      expect(dispatchFocusedEditorCommand("editor/voice-memo")).toBe(true);
      await settle();
      expect(isRecordingAudio()).toBe(true);
      first.dispose();
      // The graph switch: graph B replaces A in this window while recording.
      resetStore();
      setGraphMeta({ root: "/graphs/B" } as never);
      loadSingle(page("Other", [blk("other-b", "in B")]));
      const b = pageByName("Other")!.roots[0];
      startEditing(b, 0);
      second = mount(() => <For each={pageByName("Other")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>);
      await settle();
      (second.root.querySelector("textarea") as HTMLTextAreaElement).focus();
      expect(dispatchFocusedEditorCommand("editor/voice-memo")).toBe(true);
      await settle();
      expect(isRecordingAudio()).toBe(false);
      expect(imported).toHaveBeenCalledOnce();
      expect(imported).toHaveBeenCalledWith("/cache/tine_memo_2.m4a", expect.stringMatching(/\.m4a$/), expect.any(Number), "/graphs/A");
      const told = toasts().find((toast) => toast.message.includes("20260929_memo.m4a"));
      expect(told?.message).toContain("graph “A”");
      expect(told?.message).toContain("not inserted");
      expect(doc.byId[b].raw).toBe("in B");
    } finally {
      setRecordingAudio(false);
      second?.dispose();
    }
  });
});
