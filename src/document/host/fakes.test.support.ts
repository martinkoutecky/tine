// Test-only fakes for the page-host client: a scriptable host port and an
// in-memory document port. Not a test file itself (no suite); imported by the
// host tests.

import type { Format, PageDto, PageKind } from "../../types";
import { HostClient, type AssetWrites, type BaselineRev, type Content, type DocumentPort, type PageFacts } from "./client";
import type { EditKinds } from "../../editKind";
import type { DiskToken, HostPort, MailAnswer, MailNotice, MailPage, MailText, OwedPage, PageMail,
  PageRefusal, PublishedNeed } from "./protocol";

export function page(name: string, text = "", rev?: string | null): PageDto {
  return { name, kind: "page", title: name, pre_block: null, format: "md", rev,
    blocks: text ? [{ id: `${name}-1`, raw: text, collapsed: false, children: [] }] : [] };
}

export function textOf(dto: PageDto | null | undefined): string {
  return (dto?.blocks ?? []).map((block) => block.raw).join("\n");
}

export const NOTICE: MailNotice = { failures: 0, saveError: false, operation: null, osError: null, draftError: false, dropped: false,
  conflictReported: false, custodyError: false, indexError: false, observeError: false, twin: null };

export type Call =
  | { cmd: "open"; id: number; name: string; kind: PageKind; path: string | null }
  | { cmd: "submit"; id: number; key: string; dto: PageDto; version: number; resolve: DiskToken | null; kinds: EditKinds }
  | { cmd: "move"; id: number; source: [string, PageDto, number]; receiver: [string, PageDto, number] }
  | { cmd: "discard"; id: number; key: string; version: number }
  | { cmd: "close"; id: number; key: string }
  | { cmd: "saveNow"; keys: string[] }
  | { cmd: "wait"; needs: PublishedNeed[] }
  | { cmd: "owed"; paths: readonly string[] | null };

export class FakeHost implements HostPort {
  calls: Call[] = [];
  session = 7;
  nextId = 1;
  /** When false, command replies wait for `reply(id)`. */
  autoAdmit = true;
  private replies = new Map<number, (value: unknown) => void>();
  private replyValues = new Map<number, unknown>();
  baselineEntry = true;
  refuse: PageRefusal | null = null;
  owedPages: OwedPage[] = [];
  /** Runs inside `waitPublished` before it resolves (to model work arriving meanwhile). */
  onWait: ((needs: PublishedNeed[]) => boolean | null | void | Promise<boolean | null | void>) | null = null;
  /** Runs inside the second and later `owed` reads. */
  onOwed: (() => void) | null = null;
  /** Answers admitted commands with these mails (see `autoAnswer`). */
  respond: ((call: Call) => PageMail[]) | null = null;
  deliver: ((mail: PageMail) => void) | null = null;

  private answerLater(call: Call, admitted: Promise<unknown>): void {
    const respond = this.respond;
    if (!respond) return;
    void admitted.then((value) => Promise.resolve(value)).then((value) => {
      if (value && typeof value === "object" && "reason" in value) return;
      for (const mail of respond(call)) this.deliver?.(mail);
    });
  }

  keyOf(name: string): string { return `pages/${name}.md`; }

  async windowReloaded() { return { session: this.session, nextId: this.nextId }; }

  private admit<T>(id: number, value: T): Promise<T> {
    if (this.autoAdmit) return Promise.resolve(value);
    return new Promise<T>((resolve) => {
      this.replyValues.set(id, value);
      this.replies.set(id, resolve as (value: unknown) => void);
    });
  }

  /** Deliver a held command reply. */
  reply(id: number): void {
    const resolve = this.replies.get(id);
    this.replies.delete(id);
    resolve?.(this.replyValues.get(id));
  }

  open(_session: number, id: number, page: { name: string; kind: PageKind; path: string | null }) {
    this.calls.push({ cmd: "open", id, ...page });
    const admitted = this.admit(id, this.refuse ?? { key: this.keyOf(page.name), baselineEntry: this.baselineEntry });
    this.answerLater(this.calls[this.calls.length - 1], admitted);
    return admitted;
  }

  submit(_session: number, id: number, key: string, dto: PageDto, version: number, resolve: DiskToken | null, kinds: EditKinds) {
    this.calls.push({ cmd: "submit", id, key, dto, version, resolve, kinds });
    const admitted = this.admit(id, this.refuse);
    this.answerLater(this.calls[this.calls.length - 1], admitted);
    return admitted;
  }

  move(_session: number, id: number, source: [string, PageDto, number], receiver: [string, PageDto, number]) {
    this.calls.push({ cmd: "move", id, source, receiver });
    const admitted = this.admit(id, this.refuse);
    this.answerLater(this.calls[this.calls.length - 1], admitted);
    return admitted;
  }

  discard(_session: number, id: number, key: string, version: number) {
    this.calls.push({ cmd: "discard", id, key, version });
    const admitted = this.admit(id, this.refuse);
    this.answerLater(this.calls[this.calls.length - 1], admitted);
    return admitted;
  }

  close(_session: number, id: number, key: string) {
    this.calls.push({ cmd: "close", id, key });
    const admitted = this.admit(id, this.refuse);
    this.answerLater(this.calls[this.calls.length - 1], admitted);
    return admitted;
  }

  async saveNow(_session: number, keys: readonly string[]) { this.calls.push({ cmd: "saveNow", keys: [...keys] }); }

  /** `onWait` returning null models one bounded wait passing (F1). */
  async waitPublished(_session: number, needs: readonly PublishedNeed[]) {
    this.calls.push({ cmd: "wait", needs: [...needs] });
    const result = await this.onWait?.([...needs]);
    if (result === null) return null;
    // A publication proven here is no longer debt, as `page_owed` says.
    if (result !== false) this.owedPages = this.owedPages.filter((owed) =>
      !needs.some((need) => need.key === owed.key && need.version >= owed.version));
    return result !== false;
  }

  async owed(session: number, paths: readonly string[] | null) {
    this.calls.push({ cmd: "owed", paths });
    if (this.calls.filter((call) => call.cmd === "owed").length > 1) this.onOwed?.();
    return session === this.session && [...this.owedPages];
  }

  last<K extends Call["cmd"]>(cmd: K): Extract<Call, { cmd: K }> {
    const found = [...this.calls].reverse().find((call) => call.cmd === cmd);
    if (!found) throw new Error(`no ${cmd} call`);
    return found as Extract<Call, { cmd: K }>;
  }

  count(cmd: Call["cmd"]): number {
    return this.calls.filter((call) => call.cmd === cmd).length;
  }
}

interface DocPage { dto: PageDto; instance: number; path: string | null; kind: PageKind; format: Format; rev: BaselineRev }

export class FakeDoc implements DocumentPort {
  pages = new Map<string, DocPage>();
  held = new Set<string>();
  pushHeld = new Set<string>();
  tombs = new Set<string>();
  private instances = 0;

  /** Load a page as the loader would (a new instance). */
  load(dto: PageDto, path: string | null = `pages/${dto.name}.md`, rev: BaselineRev = dto.rev === undefined
    ? { kind: "unknown" } : dto.rev === null ? { kind: "no-file" } : { kind: "file", rev: dto.rev }): void {
    this.pages.set(dto.name, { dto, instance: ++this.instances, path, kind: dto.kind, format: dto.format ?? "md", rev });
  }

  /** A user edit to the page's text (the caller also notes it with the client). */
  type(name: string, text: string): void {
    const doc = this.pages.get(name)!;
    doc.dto = { ...doc.dto, blocks: page(name, text).blocks };
  }

  text(name: string): string { return textOf(this.pages.get(name)?.dto); }

  facts(name: string): PageFacts | null {
    const doc = this.pages.get(name);
    return doc ? { instance: doc.instance, path: doc.path, kind: doc.kind, format: doc.format, rev: doc.rev } : null;
  }

  dto(name: string): PageDto | null { return this.pages.get(name)?.dto ?? null; }

  install(name: string, content: Content): void {
    const doc = this.pages.get(name);
    const dto = content.kind === "page" ? content.dto : page(name);
    if (!doc) { this.load(dto, `pages/${name}.md`); return; }
    doc.dto = dto;
  }

  holds(name: string): boolean { return this.held.has(name); }
  holdsPush(name: string): boolean { return this.pushHeld.has(name); }
  tombstoned(name: string): boolean { return this.tombs.has(name); }
  /** Took answers' revisions, in order. */
  readonly tookRevs: [string, string][] = [];
  took(name: string, rev: string): void { this.tookRevs.push([name, rev]); }
}

export class FakeAssets implements AssetWrites {
  writes = new Set<Promise<unknown>>();
  count = 0;
  pending() { return [...this.writes]; }
  started() { return this.count; }
  /** Start a write that finishes when the returned function is called. */
  start(then?: () => void): () => void {
    let finish!: () => void;
    const write = new Promise<void>((resolve) => { finish = resolve; }).then(() => { this.writes.delete(write); then?.(); });
    this.writes.add(write);
    this.count += 1;
    return finish;
  }
}

export function mailPage(version: number, text: MailText, extra: Partial<MailPage> = {}): MailPage {
  return { version, conflict: false, risk: false, disk: null, text, ...extra };
}

export function mail(session: number, key: string, mailPageValue: MailPage | null, answer: MailAnswer | null = null): PageMail {
  return { session, key, page: mailPageValue, answer, notice: NOTICE };
}

export function took(id: number, version: number): MailAnswer {
  return { id, version, took: true, outcome: { kind: "applied" } };
}

export function applied(id: number, version: number): MailAnswer {
  return { id, version, took: false, outcome: { kind: "applied" } };
}

export async function tick(times = 5): Promise<void> {
  for (let i = 0; i < times; i += 1) await Promise.resolve();
}

/** A bound client over fresh fakes. */
export async function setup(): Promise<{ host: FakeHost; doc: FakeDoc; assets: FakeAssets; client: HostClient }> {
  const host = new FakeHost();
  const doc = new FakeDoc();
  const assets = new FakeAssets();
  const client = new HostClient(host, doc, assets);
  await client.bind();
  return { host, doc, assets, client };
}

/** Acquire `name`, answer its Open with the page at `version` (disk = buffer = `rev`). */
export async function opened(ctx: { host: FakeHost; doc: FakeDoc; client: HostClient }, name: string, version: number,
  text = "a", rev = `r${version}`): Promise<() => void> {
  if (!ctx.doc.pages.has(name)) ctx.doc.load(page(name, text, rev));
  const release = ctx.client.acquire(name);
  await tick();
  const open = ctx.host.last("open");
  ctx.client.receive(mail(ctx.host.session, ctx.host.keyOf(name),
    mailPage(version, { kind: "page", dto: page(name, text, rev) }, { disk: { kind: "file", rev } }), applied(open.id, version)));
  return release;
}

/** Make the host answer every admitted command as an ordinary healthy host would:
 * an Open reads the document's page (disk = buffer), a submit or move takes its
 * input at the next version, a close releases. Versions count from 100. */
export function autoAnswer(ctx: { host: FakeHost; doc: FakeDoc; client: HostClient }): { version: () => number } {
  let version = 100;
  // What each key's file holds after the last publication the fake host made.
  const disk = new Map<string, string>();
  const { host, doc, client } = ctx;
  host.deliver = (m) => client.receive(m);
  host.respond = (call) => {
    const s = host.session;
    switch (call.cmd) {
      case "open": {
        const key = host.keyOf(call.name);
        const current = doc.dto(call.name);
        const rev = disk.get(key) ?? (typeof current?.rev === "string" ? current.rev : `r${version + 1}`);
        version += 1;
        const dto = { ...(current ?? page(call.name)), rev };
        return [mail(s, key, mailPage(version, { kind: "page", dto }, { disk: { kind: "file", rev } }), applied(call.id, version))];
      }
      case "submit":
        version += 1;
        disk.set(call.key, `r${version}`);
        // The host holds what it took until it publishes, after a close too.
        host.owedPages.push({ key: call.key, version });
        return [mail(s, call.key, mailPage(version, { kind: "unchanged", rev: `r${version}` }), took(call.id, version))];
      case "move":
        version += 1;
        disk.set(call.source[0], `r${version}`);
        disk.set(call.receiver[0], `r${version}`);
        host.owedPages.push({ key: call.source[0], version }, { key: call.receiver[0], version });
        return [call.source[0], call.receiver[0]].map((key) =>
          mail(s, key, mailPage(version, { kind: "unchanged", rev: `r${version}` }), took(call.id, version)));
      // A close is never answered (model upClose): the host only unsubscribes.
      default:
        return [];
    }
  };
  return { version: () => version };
}
