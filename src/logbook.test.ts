import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./render/parse";
import { logbookInfo } from "./logbook";

beforeAll(async () => {
  await initParser();
});

describe("logbook wasm helpers", () => {
  it("summarizes stored CLOCK spans and exposes tooltip rows", () => {
    const info = logbookInfo(
      "DONE Ship\n:LOGBOOK:\nCLOCK: [2026-06-25 Thu 09:00:00]--[2026-06-25 Thu 09:05:00] =>  00:05:00\nCLOCK: [2026-06-25 Thu 10:00:00]--[2026-06-25 Thu 10:30:45] =>  00:30:45\n:END:",
    );
    expect(info.seconds).toBe(35 * 60 + 45);
    expect(info.summary).toBe("35m45s");
    expect(info.rows).toHaveLength(2);
    expect(info.rows[0]).toMatchObject({
      type: "CLOCK",
      start: "2026-06-25 Thu 09:00:00",
      end: "2026-06-25 Thu 09:05:00",
      span: "00:05:00",
    });
  });
});
