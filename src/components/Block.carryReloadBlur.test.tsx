import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { For, Show } from "solid-js";
import { pageByName } from "../document";
import { doc } from "../document/model";
import { installPageContent, loadSingle } from "../document/workingSet";
import { editingId, startEditing } from "../editorController";
import type { PageDto } from "../types";
import { Block } from "./Block";
import { installBlockEditorLifecycle, mount, blk, page } from "../tests/blockEditorTestkit";

// UI-CARRY-RELOAD-STALE-BLUR. An external change (Syncthing delivery, an
// external editor; the page host pushes the new text) replaces a page while a
// block on it is open in the editor. `upsertPage` carries the editor: it ends
// the edit, replaces the page's nodes and reopens the editor on the same block.
// The replacement remounts the outline, and Chromium-family webviews (Android
// WebView, Windows WebView2) fire `blur` on the removed, focused textarea.
// That blur belongs to an editor instance whose block was replaced under it:
// it must neither write its pre-reload text into the reloaded block (silently
// reverting the external change, which the window then saves) nor close the
// carried editor. jsdom fires no blur on removal, so the test dispatches the
// engine's blur on the removed textarea, after the replacement, as Chromium
// does during the DOM reconcile.

installBlockEditorLifecycle();
// The window has focus (the user is typing in it): the blur is not a window
// switch, so the editor takes its exit path.
beforeEach(() => { vi.spyOn(document, "hasFocus").mockReturnValue(true); });
afterEach(() => { vi.restoreAllMocks(); });

const NAME = "Carry";

function mountOutline() {
  // Keyed on the page object, as the app's outline remounts on a replacement.
  return mount(() => (
    <Show when={pageByName(NAME)} keyed>
      {(p) => <For each={p.roots}>{(id) => <Block id={id} />}</For>}
    </Show>
  ));
}

function external(blocks: PageDto["blocks"]): PageDto {
  return { ...page(NAME, blocks), rev: "external" };
}

async function carryAcross(before: string, after: string) {
  loadSingle(page(NAME, [blk("a", "Alpha"), blk("b", before)]));
  startEditing("b", 0);
  const { root, dispose } = mountOutline();
  const old = root.querySelector("textarea")!;
  expect(old).not.toBeNull();
  installPageContent(NAME, external([blk("a", "Alpha"), blk("b", after)]));
  old.dispatchEvent(new FocusEvent("blur"));
  await Promise.resolve();
  return { root, dispose };
}

describe("an editor carried across an external reload", () => {
  it("keeps an external edit of a code block and the carried editor", async () => {
    const after = "```js\nlet x = 2\n```";
    const { root, dispose } = await carryAcross("```js\nlet x = 1\n```", after);
    try {
      expect(doc.byId.b.raw).toBe(after);
      expect(editingId()).toBe("b");
      expect(root.querySelector("textarea")?.value).toBe("let x = 2");
    } finally {
      dispose();
    }
  });

  it("keeps an external edit of a block whose planning line exit-normalization would move", async () => {
    const after = "Bravo EXTERNAL\nmore\nSCHEDULED: <2026-10-12 Mon>\ntail";
    const { dispose } = await carryAcross("Bravo\nmore\nSCHEDULED: <2026-10-12 Mon>\ntail", after);
    try {
      expect(doc.byId.b.raw).toBe(after);
      expect(editingId()).toBe("b");
    } finally {
      dispose();
    }
  });

  it("keeps the carried editor open on a plain block", async () => {
    const { dispose } = await carryAcross("Bravo", "Bravo EXTERNAL");
    try {
      expect(doc.byId.b.raw).toBe("Bravo EXTERNAL");
      expect(editingId()).toBe("b");
    } finally {
      dispose();
    }
  });

  it("a stale editor whose block left the page neither throws nor writes", async () => {
    loadSingle(page(NAME, [blk("a", "Alpha"), blk("b", "Bravo")]));
    startEditing("b", 0);
    const { root, dispose } = mountOutline();
    try {
      const old = root.querySelector("textarea")!;
      // The external change deletes the edited block (no `id::`): the editor
      // is carried by outline position, and position 1 no longer exists.
      installPageContent(NAME, external([blk("a", "Alpha EXTERNAL")]));
      // A listener's throw is reported to the window, not to dispatchEvent.
      const thrown: unknown[] = [];
      const onError = (e: ErrorEvent) => { thrown.push(e.error); e.preventDefault(); };
      window.addEventListener("error", onError);
      try { old.dispatchEvent(new FocusEvent("blur")); } finally { window.removeEventListener("error", onError); }
      expect(thrown).toEqual([]);
      expect(doc.byId.a.raw).toBe("Alpha EXTERNAL");
      expect(doc.byId.b).toBeUndefined();
    } finally {
      dispose();
    }
  });
});
