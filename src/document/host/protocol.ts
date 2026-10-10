// The page host's wire protocol as the window client sees it (STEP3-DESIGN §3).
// Mail and refusal shapes mirror `crates/tine-store/src/page_host/binding.rs`
// (`DiskToken`, `MailText`, `MailPage`, `MailAnswer`, `MailNotice`, `PageMail`,
// `PageRefusal`) and `page_host/mod.rs` (`Refusal`). `session` replaces the
// binding+generation pair (plan v3 §5, R8). `wiring.ts` implements `HostPort`
// over the `page_*` commands (`src-tauri/src/page_commands.rs`).

import type { EditKinds } from "../../editKind";
import type { PageDto, PageKind } from "../../types";

/** The host's "this request carries no current version" (`binding.rs` `STALE`):
 * input sent on it is admitted as stale and becomes a conflict, never a write
 * over text it did not see. Host versions start at 1. */
export const STALE_VERSION = 0;

/** A disk state named by revision (`binding.rs` `DiskToken`). */
export type DiskToken = { kind: "no-file" } | { kind: "file"; rev: string };

export type MailText =
  /** Exactly the state the window submitted, or a notice-only mail. `rev` (P1)
   * names the buffer's bytes after a took answer, so the window's baseline can
   * advance with the text the host took (S5). */
  | { kind: "unchanged"; rev?: string }
  | { kind: "no-file" }
  | { kind: "page"; dto: PageDto }
  | { kind: "unreadable"; message: string };

export interface MailPage {
  /** The host version of the buffer. */
  version: number;
  conflict: boolean;
  risk: boolean;
  /** The last observed disk state (an observation; the DTO's `rev` names the buffer). */
  disk?: DiskToken | null;
  text: MailText;
}

/** Why an admitted request did not take its input (`page_host/mod.rs` `Refusal`). */
export type AnswerRefusal = "not-held" | "read-failed" | "stale" | "draft-failed";

export interface MailAnswer {
  id: number;
  /** The page version the answer leaves. */
  version: number;
  /** The request's input became the buffer. */
  took: boolean;
  outcome: { kind: "applied" } | { kind: "refused"; reason: AnswerRefusal };
}

export interface MailNotice {
  failures: number;
  saveError: boolean;
  /** The latest failed save's platform step, a fixed backend literal (never
   * a path, I-5), and its OS error code, when known (GH #538, Q-P2b-3). */
  operation: string | null;
  osError: number | null;
  draftError: boolean;
  /** A rename or delete reported as unconfirmed did not happen (Q2). */
  dropped: boolean;
  conflictReported: boolean;
  custodyError: boolean;
  indexError: boolean;
  observeError: boolean;
  twin?: string | null;
}

export interface PageMail {
  session: number;
  key: string;
  /** None once the host released the page. */
  page: MailPage | null;
  answer: MailAnswer | null;
  notice: MailNotice;
}

/** A command the host did not admit: nothing was sent (`binding.rs` `PageRefusal`). */
export type PageRefusal =
  | { reason: "invalid-target"; message: string }
  | { reason: "read-only"; message: string }
  | { reason: "twin"; existing: string }
  | { reason: "undecodable" }
  | { reason: "unreadable-owner"; file: string }
  | { reason: "failed"; message: string }
  | { reason: "not-admitted" }
  /** Client side, before any command: the page's name is an alias of these
   * existing files, so its text belongs to their owner (`resolve_page`). */
  | { reason: "alias"; owners: string[] };

/** A host page operation's disposition (`binding.rs` `PageOperation`): done
 * (published); applied but unpublished until the user acts (a conflict or a
 * persistent save error; it completes once it can); the page is busy (retry
 * once it is clean); refused (unsaved input, a stopped host, another session,
 * or a failed draft: nothing changed); not confirmed within the bound (Tine
 * keeps trying); or superseded (a later edit or discard changed the page
 * before it published). */
export type PageOperation = "applied" | "pending" | "waiting" | "refused" | "uncertain" | "superseded";

/** Crash-draft storage status at graph open (`io.rs` `DraftStatus`):
 * `unavailable` is why draft I/O is down; `unreadable` names draft files left in
 * place untouched; `unsaved` names the pages whose text is not on disk yet
 * (SPEC-s2 §4.11, STEP3 §9, B-QA). */
export interface DraftStatus {
  unavailable?: string | null;
  unreadable: string[];
  unsaved?: UnsavedPage[];
}

/** A page whose text is not on disk yet (`io.rs` `UnsavedPage`). `recovered`:
 * launch recovered it from a crash-recovery copy and Tine is saving it. */
export interface UnsavedPage {
  path: string;
  recovered: boolean;
  conflict: boolean;
  failing: boolean;
}

/** Publication debt the host holds for a key (save or index), at a version. */
export interface OwedPage {
  key: string;
  version: number;
}

/** One `pages_published` entry: the key, the version needed, and optionally a
 * block id the published bytes must contain (the block-ref witness, Q2). */
export interface PublishedNeed {
  key: string;
  version: number;
  witness?: string;
}

/** The commands the client issues (P1 supplies them as Tauri commands). Every
 * mutating command carries the session and a request id above the host's
 * admitted watermark; a resolved `PageRefusal` means "not admitted". */
export interface HostPort {
  windowReloaded(): Promise<{ session: number; nextId: number }>;
  /** `path` is the file the window's text was installed from, or, for a page
   * with no file yet, null: the port resolves the file it will be created as
   * (`resolve_page`) and sends that, never null (REVIEW-3b-P1). The reply says
   * whether the path names the opened entry under the host's page identity
   * (A-H2/Q4). */
  open(session: number, id: number, page: { name: string; kind: PageKind; path: string | null }):
    Promise<{ key: string; baselineEntry: boolean } | PageRefusal>;
  submit(session: number, id: number, key: string, dto: PageDto, version: number,
    resolve: DiskToken | null, kinds: EditKinds): Promise<PageRefusal | null>;
  move(session: number, id: number, source: [string, PageDto, number], receiver: [string, PageDto, number],
    kinds: EditKinds): Promise<PageRefusal | null>;
  discard(session: number, id: number, key: string, version: number): Promise<PageRefusal | null>;
  close(session: number, id: number, key: string): Promise<PageRefusal | null>;
  // The publication queries are the session's (REVIEW-3b-P1 F1): a result never
  // vouches for another session's pages.
  /** Make these keys' saves due now (a scheduling hint; no answer). */
  saveNow(session: number, keys: readonly string[]): Promise<void>;
  /** True once every entry is published (and indexed) at ≥ its version; false on
   * a terminal notice for a needed key or once the session is not current; null
   * when one bounded wait (≤ 5 s) passed first: the client asks again. */
  waitPublished(session: number, needs: readonly PublishedNeed[]): Promise<boolean | null>;
  /** Publication debt; `paths` (graph-relative files) filters through the host's
   * page identity, null lists all. Bounded by held pages; no filesystem scan.
   * False when there is no answer (the session is not current, or the read
   * failed): the debt is unknown, and every barrier fails on it. */
  owed(session: number, paths: readonly string[] | null): Promise<OwedPage[] | false>;
}
