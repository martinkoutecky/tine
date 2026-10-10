// Test-only: a page host over a test's fake disk, for tests whose property is
// what reaches a file (or that nothing does). It answers the `page_*` commands
// as the native host's model does for one window: an Open reads the file (disk =
// buffer), a submit on the version the page was opened at writes it, a submit or move on
// any other version (or after the file changed since the Open) is a stale
// conflict that writes nothing, a discard rereads the file, a barrier on a conflicted
// or failed page (or whose file lacks the barrier's witness) fails, and such a
// page stays owed. Not a
// test file itself.

import { vi } from "vitest";
import { backend } from "./backend";
import type { TestHost } from "./document/host/wiring.test.support";
import type { EditKinds } from "./editKind";
import type { PageDto } from "./types";

export interface FakeDisk {
  /** The file at `key` (with its `rev`), or null when there is none. */
  read(key: string, name: string): PageDto | null;
  /** Write `dto` at `key`; returns the new revision. Throwing fails the save. */
  write(key: string, dto: PageDto, kinds: EditKinds): string;
}

interface Held { name: string; version: number; rev: string | null; conflict: boolean; failed: boolean }

export function installDiskHost(host: TestHost, disk: FakeDisk) {
  const b = backend();
  const held = new Map<string, Held>();
  let version = 0;
  const rev = (key: string, name: string) => disk.read(key, name)?.rev ?? null;
  const later = (fn: () => void) => queueMicrotask(fn);
  const view = (key: string, page: Held, dto: PageDto | null, text?: "unchanged") => ({
    version: page.version, conflict: page.conflict, risk: page.conflict,
    // What the host last observed on disk (a changed file shows as the conflict's disk).
    disk: (current => current === null ? { kind: "no-file" as const } : { kind: "file" as const, rev: current })(rev(key, page.name)),
    text: text === "unchanged" ? { kind: "unchanged" as const, rev: page.rev ?? undefined }
      : dto ? { kind: "page" as const, dto } : { kind: "no-file" as const },
  });
  const took = (id: number, key: string, state: Held) => later(() => host.deliver({ key, page: view(key, state, null, "unchanged"),
    answer: { id, version: state.version, took: true, outcome: { kind: "applied" } },
    notice: { conflictReported: state.conflict, saveError: state.failed, failures: state.failed ? 3 : 0 } }));
  const spies = {
    open: vi.spyOn(b, "pageOpen").mockImplementation(async (_session, id, page) => {
      let key = page.path;
      if (key === null) {
        const resolved = await b.resolvePage(page.name, page.kind);
        if (resolved.kind === "alias") return { reason: "alias" as const, owners: resolved.owners };
        key = resolved.id;
      }
      const dto = disk.read(key, page.name);
      const state: Held = { name: page.name, version: ++version, rev: dto?.rev ?? null, conflict: false, failed: false };
      held.set(key, state);
      const at = key;
      later(() => host.deliver({ key: at, page: view(at, state, dto),
        answer: { id, version: state.version, took: false, outcome: { kind: "applied" } } }));
      return { key, baselineEntry: true };
    }),
    submit: vi.spyOn(b, "pageSubmit").mockImplementation(async (_session, id, key, dto, at, resolve, kinds) => {
      const state = held.get(key);
      if (!state) return { reason: "not-admitted" as const };
      const current = rev(key, state.name);
      // A resolving submit (keep mine) writes over exactly the disk state the
      // conflict showed; anything else must be on the current version and file.
      const stale = resolve
        ? (resolve.kind === "no-file" ? current !== null : resolve.rev !== current)
        : at !== state.version || current !== state.rev;
      state.version = ++version;
      if (stale) state.conflict = true;
      else {
        try { state.rev = disk.write(key, dto, kinds); state.conflict = false; } catch { state.failed = true; }
      }
      took(id, key, state);
      return null;
    }),
    // A move on any but the current versions is refused whole (`Refusal::Stale`):
    // nothing is taken. Otherwise the host takes both texts, the receiver's
    // drafted first (STEP3 §8; the draft store is not modelled here), and each
    // half then publishes like a submit: receiver first, a file changed since
    // the Open conflicts that half, a failing write leaves it unsaved.
    move: vi.spyOn(b, "pageMove").mockImplementation(async (_session, id, source, receiver, kinds) => {
      const halves = [receiver, source].map(([key, dto, at]) => ({ key, dto, at, state: held.get(key) }));
      if (halves.some((half) => !half.state)) return { reason: "not-admitted" as const };
      if (halves.some((half) => half.at !== half.state!.version)) {
        for (const half of halves) later(() => host.deliver({ key: half.key, page: view(half.key, half.state!, null, "unchanged"),
          answer: { id, version: half.state!.version, took: false, outcome: { kind: "refused", reason: "stale" } } }));
        return null;
      }
      for (const half of halves) {
        const state = half.state!;
        state.version = ++version;
        if (rev(half.key, state.name) !== state.rev) state.conflict = true;
        else try { state.rev = disk.write(half.key, half.dto, kinds); } catch { state.failed = true; }
      }
      for (const half of halves) took(id, half.key, half.state!);
      return null;
    }),
    // Discard (`RequestKind::Discard`): the buffer becomes the file's bytes at a
    // new version, clean; nothing is written and nothing stays owed.
    discard: vi.spyOn(b, "pageDiscard").mockImplementation(async (_session, id, key) => {
      const state = held.get(key);
      if (!state) return { reason: "not-admitted" as const };
      const dto = disk.read(key, state.name);
      Object.assign(state, { version: ++version, rev: dto?.rev ?? null, conflict: false, failed: false });
      later(() => host.deliver({ key, page: view(key, state, dto),
        answer: { id, version: state.version, took: false, outcome: { kind: "applied" } } }));
      return null;
    }),
    close: vi.spyOn(b, "pageClose").mockImplementation(async (_session, id, key) => {
      later(() => host.deliver({ key, page: null, answer: { id, version: 0, took: false, outcome: { kind: "applied" } } }));
      return null;
    }),
    // A witness (a block-ref target's id) must be in the published file's text.
    wait: vi.spyOn(b, "pageWait").mockImplementation(async (_session, needs) =>
      needs.every((need) => {
        const state = held.get(need.key);
        if (state && (state.conflict || state.failed)) return false;
        return !need.witness || JSON.stringify(state ? disk.read(need.key, state.name) : null).includes(need.witness);
      })),
    // A page whose save failed or conflicted still owes its buffer to its file.
    owed: vi.spyOn(b, "pageOwed").mockImplementation(async (_session, paths) =>
      [...held].filter(([key, state]) => (state.conflict || state.failed) && (!paths || paths.includes(key)))
        .map(([key, state]) => ({ key, version: state.version }))),
  };
  return spies;
}
