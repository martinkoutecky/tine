// A controlled transport, not a host implementation. Every command and event is
// JSON from the real Rust binding. Kept outside production builds by test imports.
import { spawn, spawnSync, type ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { resolve } from "node:path";
import { setTimeout as deadline, clearTimeout as clearDeadline } from "node:timers";
import type { GraphAnswersChange } from "../../backend";
import type { PageDto } from "../../types";
import type { PageMail } from "./protocol";

// The development agent uses RTK. Ordinary contributors/CI need only the
// repository Rust toolchain; RTK is not a new required test dependency.
const hasRtk = spawnSync("rtk", ["--version"], { stdio: "ignore" }).status === 0;
function invocation(command: string, args: string[]): [string, string[]] {
  return hasRtk ? ["rtk", ["proxy", command, ...args]] : [command, args];
}

export const TARGET = "12345678-1234-4234-8234-123456789abc";
export interface NativeImage {
  rev: string;
  counts: Record<string, number>;
  query: object;
  pages: { path: string; dto: PageDto }[];
}
export interface NativeReply {
  reply: unknown;
  custody: boolean;
  bytes: string | null;
}

export function buildPeer(): string {
  mkdirSync("artifacts/conformance-f1-f6", { recursive: true });
  const result = spawnSync(...invocation("bash", ["-c",
    'source scripts/env.sh; CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}" CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" cargo test -p tine-store --lib --no-run --message-format=json']),
  { encoding: "utf8", maxBuffer: 16 * 1024 * 1024, timeout: 600_000 });
  writeFileSync("artifacts/conformance-f1-f6/peer-build.log", result.stderr ?? "");
  if (result.status !== 0) throw new Error(`native peer build failed: ${result.stderr}\n${result.error ?? ""}`);
  const artifact = result.stdout.split("\n").filter(line => line.startsWith("{"))
    .map(line => JSON.parse(line)).find(row => row.reason === "compiler-artifact"
      && row.target.name === "tine_store" && row.profile.test && row.executable);
  if (!artifact) throw new Error("missing native lib-test executable");
  return artifact.executable;
}

export class NativePeer {
  private child: ChildProcessWithoutNullStreams;
  private pending: { resolve: (value: unknown) => void; reject: (error: Error) => void }[] = [];
  private stderr = "";
  private ended: Error | undefined;
  readonly trace: unknown[] = [];
  private readonly commit = spawnSync(...invocation("git", ["rev-parse", "HEAD"]), { encoding: "utf8" }).stdout?.trim();
  constructor(executable: string) {
    this.child = spawn(...invocation(executable, [
      "page_host::binding::tests::window::native_window_peer", "--exact", "--ignored", "--nocapture"]),
    { cwd: resolve("."), stdio: "pipe" });
    this.child.stderr.on("data", chunk => { this.stderr += String(chunk); });
    createInterface({ input: this.child.stdout }).on("line", line => {
      const marker = line.indexOf("TINE_WINDOW_JSON ");
      if (marker < 0) return; // libtest may print its test label before the first frame.
      const value: unknown = JSON.parse(line.slice(marker + "TINE_WINDOW_JSON ".length));
      this.trace.push({ received: value });
      this.pending.shift()?.resolve(value);
    });
    this.child.on("exit", code => {
      this.ended = new Error(`native peer exited ${code}: ${this.stderr}`);
      for (const request of this.pending.splice(0)) request.reject(new Error(`native peer exited ${code}: ${this.stderr}`));
    });
  }
  rpc<T = unknown>(arg: Record<string, unknown>): Promise<T> {
    this.trace.push({ sent: arg });
    return new Promise((resolve, reject) => {
      if (this.ended) { reject(this.ended); return; }
      const timeout = deadline(() => reject(new Error(`native timeout ${JSON.stringify(arg)}: ${this.stderr}`)), 15000);
      this.pending.push({ resolve: value => { clearDeadline(timeout); resolve(value as T); },
        reject: error => { clearDeadline(timeout); reject(error); } });
      this.child.stdin.write(`${JSON.stringify(arg)}\n`);
    });
  }
  mail(answer?: number): Promise<PageMail[]> { return this.rpc({ op: "mail", answer }); }
  image(fresh = false): Promise<NativeImage> { return this.rpc({ op: fresh ? "fresh" : "image" }); }
  changes(): Promise<GraphAnswersChange[]> { return this.rpc({ op: "changes" }); }
  async close(): Promise<void> {
    if (this.ended) return;
    this.child.stdin.end();
    await new Promise<void>((resolve, reject) => this.child.once("exit", code => code === 0
      ? resolve() : reject(new Error(`native peer exited ${code}: ${this.stderr}`))));
  }
  capsule(seed: number, error: unknown): void {
    writeFileSync(`artifacts/conformance-f1-f6/failure-${seed}.json`, JSON.stringify({
      testedCommit: this.commit, seed, journey: "real window custody / native publications / cold frontend mirrors",
      expected: "model window obligations and independent cold native answers agree at the checked boundary",
      classification: "ambiguous until diagnosed", observed: String(error), stderr: this.stderr, trace: this.trace,
    }, null, 2));
  }
}

export function random(seed: number): (n: number) => number {
  let value = seed || 1;
  return n => { value ^= value << 13; value ^= value >>> 17; value ^= value << 5; return (value >>> 0) % n; };
}

/** Native runtime ids and temp-root spellings are incidental. Preserve every
 * semantic DTO field, raw byte, outline edge and multiplicity. */
export function semantics(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(semantics);
  if (value && typeof value === "object") return Object.fromEntries(Object.entries(value)
    .filter(([key]) => !["id", "rev", "mtime"].includes(key))
    .map(([key, item]) => [key, semantics(item)]));
  if (typeof value === "string") return value.replace(/[^"\s]*\/graph\/pages\//g, "pages/");
  return value;
}
