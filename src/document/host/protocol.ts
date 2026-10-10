// The page host's wire protocol as the window client sees it (STEP3-DESIGN §3).
// Mail and refusal shapes mirror `crates/tine-store/src/page_host/binding.rs`
// (`DiskToken`, `MailText`, `MailPage`, `MailAnswer`, `MailNotice`, `PageMail`,
// `PageRefusal`) and `page_host/mod.rs` (`Refusal`). `session` replaces the
// binding+generation pair (plan v3 §5, R8); the commands it qualifies land in P1.
//
// Step 3b P2a: nothing in production imports this folder yet
// (`boundary.guard.test.ts`, "P2a client is unwired").

import type { EditKinds } from "../../editKind";
import type { PageDto } from "../../types";

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
  draftError: boolean;
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
  | { reason: "not-admitted" };

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
  /** `baseline` is the file the window's text was installed from; the reply says
   * whether it names the opened entry under the host's page identity (A-H2/Q4). */
  open(session: number, id: number, page: { name: string; path: string | null }):
    Promise<{ key: string; baselineEntry: boolean } | PageRefusal>;
  submit(session: number, id: number, key: string, dto: PageDto, version: number,
    resolve: DiskToken | null, kinds: EditKinds): Promise<PageRefusal | null>;
  move(session: number, id: number, source: [string, PageDto, number], receiver: [string, PageDto, number],
    kinds: EditKinds): Promise<PageRefusal | null>;
  discard(session: number, id: number, key: string, version: number): Promise<PageRefusal | null>;
  close(session: number, id: number, key: string): Promise<PageRefusal | null>;
  /** Make these keys' saves due now (a scheduling hint; no answer). */
  saveNow(keys: readonly string[]): Promise<void>;
  /** True once every entry is published (and indexed) at ≥ its version; false on
   * a terminal notice for a needed key or at the caller's bound. */
  waitPublished(needs: readonly PublishedNeed[]): Promise<boolean>;
  /** Publication debt; `paths` (graph-relative files) filters through the host's
   * page identity, null lists all. Bounded by held pages; no filesystem scan. */
  owed(paths: readonly string[] | null): Promise<OwedPage[]>;
}
