import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { gitLifecycleEdge } from "./gitLifecycle";

function harness(flush: () => Promise<boolean>) {
  const log: string[] = [];
  return {
    log,
    flushAll: async () => {
      log.push("flush:start");
      const saved = await flush();
      log.push("flush:end");
      return saved;
    },
    tell: async (foreground: boolean) => {
      log.push(`tell:${foreground}`);
    },
  };
}

describe("git lifecycle edges (ADR 0078 slice 3)", () => {
  it("flushes the pages before the background commit is asked for", async () => {
    const h = harness(async () => true);
    await gitLifecycleEdge(false, h.flushAll, h.tell);
    const log = h.log;
    expect(log).toEqual(["flush:start", "flush:end", "tell:false"]);
  });

  it("still commits what was saved when the flush fails", async () => {
    const h = harness(async () => { throw new Error("disk"); });
    await gitLifecycleEdge(false, h.flushAll, h.tell);
    const log = h.log;
    expect(log).toEqual(["flush:start", "tell:false"]);
  });

  it("comes back to the foreground without a flush", async () => {
    const h = harness(async () => true);
    await gitLifecycleEdge(true, h.flushAll, h.tell);
    const log = h.log;
    expect(log).toEqual(["tell:true"]);
  });

  it("never rejects when the backend refuses", async () => {
    await expect(gitLifecycleEdge(true, async () => true, () => Promise.reject(new Error("refused"))))
      .resolves.toBeUndefined();
  });

  it("drops a background edge a foreground edge overtook during the flush", async () => {
    let finishFlush!: () => void;
    const log: string[] = [];
    const tell = async (fg: boolean) => { log.push(`tell:${fg}`); };
    const away = gitLifecycleEdge(false, () => new Promise<boolean>((done) => {
      finishFlush = () => done(true);
    }), tell);
    await gitLifecycleEdge(true, async () => true, tell);
    finishFlush();
    await away;
    expect(log).toEqual(["tell:true"]);
  });

  it("sends edges in order when none overtakes another", async () => {
    const log: string[] = [];
    const tell = async (fg: boolean) => { log.push(`tell:${fg}`); };
    await gitLifecycleEdge(true, async () => true, tell);
    await gitLifecycleEdge(false, async () => true, tell);
    await gitLifecycleEdge(true, async () => true, tell);
    expect(log).toEqual(["tell:true", "tell:false", "tell:true"]);
  });

  it("follows the mobile session-activity edges in App", () => {
    const app = readFileSync("src/App.tsx", "utf8");
    const install = app.slice(app.indexOf("installSessionActivity({"));
    const body = install.slice(0, install.indexOf("})));"));
    expect(body).toContain("isMobile: isMobilePlatform");
    expect(body).toContain("gitLifecycleEdge(active, flushEverything)");
  });
});
