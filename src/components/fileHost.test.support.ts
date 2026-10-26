// Test-only: a page host over an in-memory file map, on the backend's `page_*`
// seam (not a test file itself). Unlike the browser mock's host (which never
// writes), a submit here becomes the key's file at a fresh revision, and an Open
// reads the file back with the host version it is at, so a test can follow a
// page across creation, reopen and later edits. Mail goes through the window's
// real listener (`bindTestHost`), addressed to the window's current session, so a
// test may rebind (a graph load) after this helper bound.

import { vi, type MockInstance } from "vitest";
import { backend, type Backend } from "../backend";
import type { PageDto } from "../types";
import { bindTestHost, type TestHost } from "../document/host/wiring.test.support";
import type { MailPage } from "../document/host/protocol";

export interface FileHost {
  host: TestHost;
  /** Each key's file: its page and revision. A test may seed or replace entries. */
  files: Map<string, { dto: PageDto; rev: string }>;
  /** The host version of each key's buffer. */
  versions: Map<string, number>;
  submit: MockInstance<Backend["pageSubmit"]>;
  open: MockInstance<Backend["pageOpen"]>;
}

/** Bind the window and serve its page commands from `files`. `rev(key, n)` names
 * the n-th written revision (default `rev-<n>`). */
export async function bindFileHost(rev: (key: string, n: number) => string = (_key, n) => `rev-${n}`): Promise<FileHost> {
  const host = await bindTestHost();
  const b = backend();
  let session = host.session;
  const reloaded = b.pageWindowReloaded.bind(b);
  vi.spyOn(b, "pageWindowReloaded").mockImplementation(async () => {
    const reply = await reloaded();
    session = reply.session;
    return reply;
  });
  // `bindTestHost`'s mail carries its bound session unless the mail names one.
  type Mail = Parameters<TestHost["deliver"]>[0];
  const deliver = (mail: Mail) => host.deliver({ ...mail, session });
  const files = new Map<string, { dto: PageDto; rev: string }>();
  const versions = new Map<string, number>();
  let clock = 0;
  let written = 0;
  const view = (key: string): MailPage => {
    const file = files.get(key);
    if (!versions.has(key)) versions.set(key, ++clock);
    return { version: versions.get(key)!, conflict: false, risk: false,
      disk: file ? { kind: "file", rev: file.rev } : { kind: "no-file" },
      text: file ? { kind: "page", dto: { ...structuredClone(file.dto), rev: file.rev } } : { kind: "no-file" } };
  };
  const later = (fn: () => void) => queueMicrotask(fn);
  const open = vi.spyOn(b, "pageOpen").mockImplementation(async (_session, id, page) => {
    let key = page.path;
    if (key === null) {
      const resolved = await b.resolvePage(page.name, page.kind);
      if (resolved.kind === "alias") return { reason: "alias", owners: resolved.owners };
      key = resolved.id;
    }
    const at = key;
    later(() => {
      const state = view(at);
      deliver({ key: at, page: state, answer: { id, version: state.version, took: false, outcome: { kind: "applied" } } });
    });
    return { key: at, baselineEntry: true };
  });
  const submit = vi.spyOn(b, "pageSubmit").mockImplementation(async (_session, id, key, dto) => {
    const next = rev(key, ++written);
    files.set(key, { dto: structuredClone(dto), rev: next });
    const version = ++clock;
    versions.set(key, version);
    later(() => deliver({ key, answer: { id, version, took: true, outcome: { kind: "applied" } },
      page: { version, conflict: false, risk: false, disk: { kind: "file", rev: next }, text: { kind: "unchanged", rev: next } } }));
    return null;
  });
  vi.spyOn(b, "pageClose").mockImplementation(async (_session, id, key) => {
    later(() => deliver({ key, page: null, answer: { id, version: 0, took: false, outcome: { kind: "applied" } } }));
    return null;
  });
  return { host, files, versions, submit, open };
}
