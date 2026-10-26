// Test-only support for component tests that need a page the window's page host
// holds (open, conflicted) without typing into it. Not a test file itself.

import { vi } from "vitest";
import { backend } from "../backend";
import { endEdit, startEditing } from "../editorController";
import { pageByName } from "../document";
import { baseRevFor, hostHolds, isSaving } from "../document/host/wiring";
import type { DiskToken } from "../document/host/protocol";
import type { TestHost } from "../document/host/wiring.test.support";

/** Answer every Open as a host that read the file the window shows: the answer
 * carries no new text (nothing is installed), at host version 1. Returns the spy. */
export function openAsLoaded(host: TestHost) {
  return vi.spyOn(backend(), "pageOpen").mockImplementation(async (_session, id, page) => {
    const key = page.path ?? `${page.kind === "journal" ? "journals" : "pages"}/${page.name}.md`;
    const rev = baseRevFor(page.name);
    const disk: DiskToken = rev ? { kind: "file", rev } : { kind: "no-file" };
    queueMicrotask(() => host.deliver({ key, answer: { id, version: 1, took: false, outcome: { kind: "applied" } },
      page: { version: 1, conflict: false, risk: false, disk, text: { kind: "unchanged" } } }));
    return { key, baselineEntry: true };
  });
}

/** Open `name` through edit intent (a block of it enters editing), as the UI does. */
export async function openInHost(name: string): Promise<void> {
  const root = pageByName(name)?.roots[0];
  if (!root) throw new Error(`openInHost: ${name} has no block to edit`);
  startEditing(root);
  // Held and nothing outstanding: the Open was answered.
  await vi.waitFor(() => { if (!hostHolds(name) || isSaving(name)) throw new Error(`${name} not open in the host`); });
}

/** The host reports a disk conflict on `key` (its custody applied: `conflictReported`);
 * a null `diskRev` is a file deleted on disk. */
export function reportConflict(host: TestHost, key: string, diskRev: string | null = "disk-2", version = 2): void {
  const disk: DiskToken = diskRev === null ? { kind: "no-file" } : { kind: "file", rev: diskRev };
  host.deliver({ key, answer: null, notice: { conflictReported: true },
    page: { version, conflict: true, risk: true, disk, text: { kind: "unchanged" } } });
}

/** Open `name` in the host and make it a reported conflict; editing ends after. */
export async function conflictedInHost(host: TestHost, name: string, key = pageByName(name)?.id ?? `pages/${name}.md`): Promise<void> {
  await openInHost(name);
  reportConflict(host, key);
  endEdit("blur");
}
