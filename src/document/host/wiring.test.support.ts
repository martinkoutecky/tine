// Test-only support for tests that drive the wired page host (`wiring.ts`)
// over the test backend's `mockPageHost` (src/mock.ts). Not a test file itself.
//
// `bindTestHost()` binds the window after a test loaded its pages (before it,
// edits are only noted) and returns a handle that can deliver host mail as the
// native host would: a save-error notice, a conflict, an external change. The
// mail listener is captured on the file's first bind (wiring listens once per
// module instance), so a file that delivers mail binds through this helper first.

import { vi } from "vitest";
import { backend } from "../../backend";
import type { PageDto } from "../../types";
import { bindHost } from "./wiring";
import type { MailNotice, MailPage, PageMail } from "./protocol";

let listener: ((mail: PageMail) => void) | null = null;
let session = 0;

export const NOTICE: MailNotice = { failures: 0, saveError: false, operation: null, osError: null, draftError: false, dropped: false,
  conflictReported: false, custodyError: false, indexError: false, observeError: false, twin: null };

export interface TestHost {
  /** The bound session (mail for another session is dropped by the client). */
  readonly session: number;
  /** Deliver one page mail to the window, as the native host's event would:
   * under the bound session unless `session` names another (a later rebind's,
   * or a stale one the client must drop). */
  deliver(mail: Omit<PageMail, "session" | "notice"> & { session?: number; notice?: Partial<MailNotice> }): void;
  /** A notice-only mail for `key` (e.g. the host's save failure after three
   * attempts): as the native host sends it, with the page state and its text
   * unchanged (`binding.rs` "a notice that changed without a page change"). */
  notice(key: string, notice: Partial<MailNotice>, page?: Partial<MailPage>): void;
}

export async function bindTestHost(): Promise<TestHost> {
  const b = backend();
  const subscribe = b.onPageMail.bind(b);
  const reloaded = b.pageWindowReloaded.bind(b);
  const onMail = vi.spyOn(b, "onPageMail").mockImplementation((cb) => {
    listener = cb;
    return subscribe(cb);
  });
  const reload = vi.spyOn(b, "pageWindowReloaded").mockImplementation(async () => {
    const reply = await reloaded();
    session = reply.session;
    return reply;
  });
  try {
    await bindHost();
  } finally {
    onMail.mockRestore();
    reload.mockRestore();
  }
  const bound = session;
  const deliver: TestHost["deliver"] = (mail) => {
    if (!listener) throw new Error("bindTestHost: the page-mail listener was registered before this helper bound");
    listener({ ...mail, session: mail.session ?? bound, notice: { ...NOTICE, ...mail.notice } });
  };
  return {
    session: bound,
    deliver,
    notice: (key, notice, page = {}) => deliver({ key, answer: null, notice,
      page: { version: 1, conflict: false, risk: false, disk: null, text: { kind: "unchanged" }, ...page } }),
  };
}

/** A host page state carrying `dto` (a push: external change, discard, load). */
export function mailPage(version: number, dto: PageDto | null, extra: Partial<MailPage> = {}): MailPage {
  return { version, conflict: false, risk: false, disk: dto?.rev ? { kind: "file", rev: dto.rev } : { kind: "no-file" },
    text: dto ? { kind: "page", dto } : { kind: "no-file" }, ...extra };
}

/** The DTOs `pageSubmit` carried for `name`, in call order. */
export function submittedPages(spy: { mock: { calls: unknown[][] } }, name?: string): PageDto[] {
  return spy.mock.calls.map((call) => call[3] as PageDto).filter((dto) => name === undefined || dto.name === name);
}
