import { onCleanup } from "solid-js";
import { backend } from "../backend";
import { pageByName, node as docNode, formatForBlock, trackAssetWrite } from "../document";
import { assetEditorIsCurrent, captureAssetEditor, importCaptureToOrigin, reportStaleAsset, type AssetEditorToken } from "../assetLanding";
import { bindingCurrent } from "../binding";
import { bindingOwner, ownedWhen, readOwned, writeOwned } from "../owned";
import { holdExternalActivity } from "../externalActivity";
import { assetMarkdown, assetFileName, captureAssetFileName, recordingExt } from "../media";
import { MEDIA_EDITORS } from "../mediaEditors";
import { resolveMediaEditorCommand } from "../mediaEditorSettings";
import { refreshAssetOnReturn } from "../assetRefresh";
import { seedAssetBlob } from "../assetCache";
import { clipboardSizeAllowed } from "../clipboardImage";
import { msg } from "../i18n";
import { pushToast, dismissToast } from "../toasts";
import { isRecordingAudio, setRecordingAudio, cancelDesktopVoiceRecording, desktopVoiceRecordingActive, startDesktopVoiceRecording, stopDesktopVoiceRecording } from "../mediaCapture";

// One recorder app-wide: Stop anywhere stores into the initiating graph.
let mobileRecordingEditorToken: AssetEditorToken | null = null;
const MAX_BYTE_CLIPBOARD_FILE = 64 * 1024 * 1024;
const MAX_CLIPBOARD_FILES = 32;

/** Shared native acquisition, durable import and owned reference landing.
 * Editor adapters supply only identity and insertion into the live selection. */
export function editorAssets(ctx: {
  id: string;
  element(): HTMLElement;
  mounted(): boolean;
  insert(markup: string, current: () => boolean): void;
}) {
  const captureAssetEditorToken = () => captureAssetEditor(ctx.element());
  const assetEditorCurrent = (token: AssetEditorToken) => assetEditorIsCurrent(token, ctx.element(), ctx.mounted());

  // Seed the preview cache, persist the bytes to assets/ (awaited: data before reference), then
  // insert a link to the STORED name at the caret (the backend may de-dup the candidate name).
  // Shared by clipboard-image paste and mobile capture (camera / voice memo).
  const insertAssetBytes = async (token: AssetEditorToken, bytes: Uint8Array, origName?: string, captureExt?: string) => {
    const owner = bindingOwner(() => assetEditorCurrent(token));
    const candidate = captureExt !== undefined ? captureAssetFileName(captureExt) : assetFileName(origName);
    // Cache key is the bare filename — assetRelPath() strips the `assets/` prefix
    // before loadAssetBlob() (see render/inline.tsx). Seed it so the asset renders
    // instantly, before the disk write lands.
    seedAssetBlob(candidate, bytes);
    let stored: string;
    try {
      // Data before reference: a crash may leave an orphan asset, but can never
      // persist a note that points at bytes which existed only in WebView memory.
      const result = await writeOwned(owner, trackAssetWrite(backend().saveAsset(candidate, bytes, token.binding.backendGeneration)));
      if (result.kind === "stale") { reportStaleAsset(); return; }
      stored = result.value;
    } catch (error) { pushToast(msg("editor-block-couldn-t-save-to-assets", { detail: String(error) }), "error"); return; }
    if (stored !== candidate) seedAssetBlob(stored, bytes);
    insertStoredAssets(token, [{ stored, label: origName }]);
  };

  const insertStoredAssets = (token: AssetEditorToken, assets: { stored: string; label?: string }[]): boolean => {
    if (!assets.length) return false;
    if (!assetEditorCurrent(token)) {
      reportStaleAsset();
      return false;
    }
    const page = pageByName(docNode(ctx.id)?.page ?? "");
    const markdown = assets.map(({ stored, label }) => assetMarkdown(stored, {
      label,
      pagePath: page?.id,
      format: formatForBlock(ctx.id),
    })).join("\n");
    ctx.insert(markdown, () => assetEditorCurrent(token));
    return true;
  };

  /** Import file-manager paths without materializing their bytes in the WebView.
   * If a platform exposes only browser File objects, save those sequentially so
   * at most one bounded byte buffer/base64 IPC payload is live at a time. */
  const pasteClipboardFiles = async (eventFiles: File[]) => {
    const editorToken = captureAssetEditorToken();
    const owner = bindingOwner();
    const toastId = pushToast(msg("editor-block-pasting-files"), "info");
    let skipped = 0;
    let nativeUnavailable = false;
    const stored: { stored: string; label?: string }[] = [];
    let inserted = false;
    try {
      const nativeResult = await readOwned(ownedWhen(() => ctx.mounted()), backend().clipboardFiles().catch(() => {
        nativeUnavailable = true;
        return { files: [], skipped: 0, truncated: false };
      }));
      if (nativeResult.kind === "stale") return; const native = nativeResult.value;
      if (native.files.length) {
        skipped += native.skipped;
        for (const file of native.files) {
          try {
            const result = await writeOwned(owner, trackAssetWrite(backend().importAsset(file.path, assetFileName(file.name), editorToken.binding.backendGeneration)));
            if (result.kind === "stale") return; stored.push({ stored: result.value, label: file.name });
          } catch (error) {
            pushToast(msg("editor-block-couldn-t-import-pasted-file", { detail: String(error) }), "error");
            skipped += 1;
          }
        }
      } else {
        const files = eventFiles.slice(0, MAX_CLIPBOARD_FILES);
        skipped += Math.max(0, eventFiles.length - files.length);
        // A content-URI File can omit MIME as well as size/bytes. Let the
        // native image reader identify it; non-image files keep generic ingress.
        if (files.length === 1 && !files[0].type && clipboardSizeAllowed(files[0].size)) {
          const result = await readOwned(bindingOwner(() => assetEditorCurrent(editorToken)), backend().readClipboardImage());
          if (result.kind === "stale") return;
          const bytes = result.value;
          if (bytes?.length && bytes.length <= MAX_BYTE_CLIPBOARD_FILE) {
            await insertAssetBytes(editorToken, bytes);
            return;
          }
        }
        // Keep the screenshot/image behavior: image-only clipboard payloads use
        // timestamp names and optimistic rendering rather than synthetic names
        // such as Chromium's generic "image.png".
        if (files.length === 1 && files[0].type.startsWith("image/")) {
          const image = files[0];
          if (!clipboardSizeAllowed(image.size)) skipped += Math.max(1, native.skipped);
          else {
            try {
              let bytes: Uint8Array | null;
              try { bytes = new Uint8Array(await image.arrayBuffer()); }
              catch { bytes = null; }
              // Android content-URI clips can expose an empty/unreadable File
              // in the WebView. Read through ContentResolver instead of treating
              // that failure as evidence that the image exceeds the budget.
              if (!bytes?.length) {
                const result = await readOwned(bindingOwner(() => assetEditorCurrent(editorToken)), backend().readClipboardImage());
                if (result.kind === "stale") return;
                bytes = result.value;
              }
              if (bytes?.length && bytes.length <= MAX_BYTE_CLIPBOARD_FILE) {
                // A Windows bitmap clipboard can appear as one invalid native
                // path plus one valid WebView2 image File. Once the bytes win,
                // suppress that native pseudo-entry's skipped count (GH #78).
                await insertAssetBytes(editorToken, bytes);
              } else skipped += Math.max(1, native.skipped);
            } catch {
              skipped += Math.max(1, native.skipped);
            }
          }
          return;
        }
        skipped += native.skipped;
        for (const file of files) {
          if (!clipboardSizeAllowed(file.size)) {
            skipped += 1;
            continue;
          }
          try {
            const bytes = new Uint8Array(await file.arrayBuffer());
            if (!bytes.length || bytes.length > MAX_BYTE_CLIPBOARD_FILE) {
              skipped += 1;
              continue;
            }
            const candidate = assetFileName(file.name || undefined);
            const result = await writeOwned(owner, trackAssetWrite(backend().saveAsset(candidate, bytes, editorToken.binding.backendGeneration)));
            if (result.kind === "stale") return; const saved = result.value;
            stored.push({ stored: saved, label: file.name || undefined });
            try {
              seedAssetBlob(saved, bytes);
            } catch {
              // The durable asset + link are authoritative; cache warming is optional.
            }
          } catch (error) {
            pushToast(msg("editor-block-couldn-t-save-pasted-file", { detail: String(error) }), "error");
            skipped += 1;
          }
        }
      }
      inserted = insertStoredAssets(editorToken, stored);
    } finally {
      dismissToast(toastId);
      if (!bindingCurrent(editorToken.binding)) return;
      if (inserted) {
        pushToast(msg("editor-block-inserted-file", { count: stored.length }), "success");
      }
      if (skipped) {
        pushToast(
          msg("editor-block-skipped-item-folders-and-byte-only-files-over-64-mib-aren-t-pasted", { count: skipped }),
          "error"
        );
      } else if (nativeUnavailable && !eventFiles.length) {
        pushToast(msg("editor-block-couldn-t-read-copied-files-use-upload-or-drag-and-drop-instead"), "error");
      }
    }
  };
  // I-21 / GH #622: the native operation owns blur through durable import and
  // reference landing, not just while the chooser promise is pending. Count
  // overlapping operations so one completion cannot retire another's ownership.
  let nativeAssetPickers = 0;
  const withNativeAssetPicker = async (work: (token: AssetEditorToken) => Promise<void>) => {
    const token = captureAssetEditorToken();
    nativeAssetPickers++;
    const release = holdExternalActivity();
    try { await work(token); } finally { nativeAssetPickers--; release(); }
  };
  // Mobile: take/pick a photo (Android camera plugin) → insert at the caret.
  const capturePhotoCmd = () => withNativeAssetPicker(async (editorToken) => {
    let res;
    try {
      res = await backend().capturePhoto();
    } catch (err) {
      if (bindingCurrent(editorToken.binding)) pushToast(msg("editor-block-couldn-t-capture-a-photo", { detail: String(err) }), "error");
      return;
    }
    if (res.status === "ok" && res.path) {
      const candidate = captureAssetFileName(res.ext || "jpg");
      try {
        const stored = await trackAssetWrite(importCaptureToOrigin(editorToken, res.path, candidate));
        if (stored) insertStoredAssets(editorToken, [{ stored }]);
      } catch (err) {
        pushToast(msg("editor-block-couldn-t-import-the-photo", { detail: String(err) }), "error");
      }
    }
  });

  // Mobile: toggle voice-memo recording. First tap starts (prompts for mic
  // permission); second tap stops and inserts the recorded audio at the caret.
  const voiceMemoToggle = async () => {
    if (isRecordingAudio()) {
      const editorToken = mobileRecordingEditorToken ?? captureAssetEditorToken();
      mobileRecordingEditorToken = null;
      setRecordingAudio(false);
      let res;
      try {
        res = await backend().stopRecording();
      } catch (err) {
        pushToast(msg("editor-block-couldn-t-save-the-recording", { detail: String(err) }), "error");
        return;
      }
      if (res.status === "ok" && res.path) {
        const candidate = captureAssetFileName(res.ext || "m4a");
        try {
          const stored = await trackAssetWrite(importCaptureToOrigin(editorToken, res.path, candidate));
          if (stored) insertStoredAssets(editorToken, [{ stored }]);
        } catch (err) {
          pushToast(msg("editor-block-couldn-t-import-the-recording", { detail: String(err) }), "error");
        }
      }
      return;
    }
    const editorToken = captureAssetEditorToken();
    let res;
    try {
      res = await backend().startRecording();
    } catch (err) {
      if (bindingCurrent(editorToken.binding)) pushToast(msg("editor-block-couldn-t-start-recording", { detail: String(err) }), "error");
      return;
    }
    if (!bindingCurrent(editorToken.binding)) { if (res.status === "recording") void backend().cancelRecording(); return; }
    if (res.status === "recording") {
      mobileRecordingEditorToken = editorToken;
      setRecordingAudio(true);
      pushToast(msg("editor-block-recording-tap-the-mic-again-to-stop"), "info");
    }
  };

  // Desktop voice memo (/record): one process-wide owner reserves the physical
  // recorder before permission, bounds time/bytes, and is cancelled if this editor
  // unmounts. This keeps the microphone reachable and prevents concurrent sessions.
  const desktopRecordingOwner = Symbol(`voice-recording:${ctx.id}`);
  onCleanup(() => cancelDesktopVoiceRecording(desktopRecordingOwner));
  const desktopVoiceMemoToggle = async () => {
    if (desktopVoiceRecordingActive()) {
      stopDesktopVoiceRecording();
      return;
    }
    const editorToken = captureAssetEditorToken();
    try {
      const status = await startDesktopVoiceRecording(desktopRecordingOwner, {
        complete: async (bytes, mime, limited) => {
          if (limited) pushToast(msg("editor-block-recording-limit-reached-saving-the-captured-audio"), "info");
          await insertAssetBytes(editorToken, bytes, undefined, recordingExt(mime));
        },
        error: (message) => pushToast(msg("editor-block-desktop-recording-save-error", { error: String(message) }), "error"),
      });
      if (status === "busy") {
        pushToast(msg("editor-block-another-voice-recording-is-already-active"), "error");
        return;
      }
      pushToast(msg("editor-block-recording-run-record-again-to-stop"), "info");
    } catch (err) {
      pushToast(msg("editor-block-couldn-t-access-the-microphone", { detail: String(err) }), "error");
    }
  };
  const uploadAsset = () => withNativeAssetPicker(async (editorToken) => {
    const owner = bindingOwner();
    const picked = await readOwned(ownedWhen(() => ctx.mounted()), backend().pickFile());
    if (picked.kind === "stale" || !picked.value) return; const path = picked.value;
    try {
      // Store with a timestamped name (keeps the original + a sortable insert time).
      const orig = path.split(/[\\/]/).pop() || undefined;
      const saved = await writeOwned(owner, trackAssetWrite(backend().importAsset(path, assetFileName(orig), editorToken.binding.backendGeneration)));
      if (saved.kind === "current") insertStoredAssets(editorToken, [{ stored: saved.value, label: orig }]);
    } catch (error) {
      pushToast(msg("editor-block-couldn-t-import-asset", { detail: String(error) }), "error");
    }
  });

  // `/drawio` creates an editable asset, inserts its reference, then opens the
  // editor. assetRefresh updates the image when Tine regains focus.
  const createDrawioDiagram = async () => {
    const editorToken = captureAssetEditorToken();
    const owner = bindingOwner(() => assetEditorCurrent(editorToken));
    const ed = MEDIA_EDITORS.find((e) => e.id === "drawio");
    if (!ed?.blank) return;
    try {
      const bytes = new TextEncoder().encode(ed.blank.contents());
      // Use the unique-stem asset convention (like a camera/mic capture), NOT a
      // fixed `diagram.drawio.svg`: the backend de-dup splits on the LAST dot, so a
      // colliding `diagram.drawio.svg` would become `diagram.drawio_1.svg` — which
      // no longer ends in `.drawio.svg`, dropping the "Edit in draw.io" affordance
      // (GH #38). A unique stem never collides, so the double extension survives.
      const savedResult = await writeOwned(owner, trackAssetWrite(
        backend().saveAsset(captureAssetFileName(ed.blank.ext), bytes, editorToken.binding.backendGeneration)
      ));
      if (savedResult.kind === "stale") return; const saved = savedResult.value;
      insertStoredAssets(editorToken, [{ stored: saved }]);
      const cmd = await resolveMediaEditorCommand(ed);
      if (!owner()) return;
      void writeOwned(owner, backend().editAssetExternal(saved, cmd, editorToken.binding.backendGeneration))
        .catch(() => pushToast(msg("editor-block-couldn-t-open-draw-io"), "error"));
      refreshAssetOnReturn(saved);
    } catch (error) {
      pushToast(msg("editor-block-couldn-t-create-the-diagram", { detail: String(error) }), "error");
    }
  };

  const pasteImage = (e: ClipboardEvent) => {
    const looksImage = (e.clipboardData?.types ?? []).some(
      (t) => t.startsWith("image/") || t === "Files"
    );
    const toastId = looksImage ? pushToast(msg("editor-block-pasting-image"), "info") : 0;
    const editorToken = captureAssetEditorToken();
    const owner = bindingOwner(() => assetEditorCurrent(editorToken));
    void (async () => {
      let bytes: Uint8Array | null = null;
      try {
        const result = await readOwned(owner, backend().readClipboardImage());
        if (result.kind === "stale") return; bytes = result.value;
      } finally {
        if (toastId) dismissToast(toastId);
      }
      if (!bytes) return;
      await insertAssetBytes(editorToken, bytes);
    })();
  };
  return { uploadAsset, capturePhotoCmd, voiceMemoToggle, desktopVoiceMemoToggle, createDrawioDiagram,
    pasteClipboardFiles, pasteImage, holdsBlur: () => nativeAssetPickers > 0 };
}
