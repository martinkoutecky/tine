import fs from "node:fs";
import { setTimeout as delay } from "node:timers/promises";

/** Wait for a persisted file's semantic content, returning its final text. */
export async function waitForFileText(file, predicate, { timeoutMs = 15_000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let last = "<absent>";
  while (Date.now() < deadline) {
    try { last = fs.readFileSync(file, "utf8"); } catch { last = "<absent>"; }
    if (predicate(last)) return last;
    await delay(100);
  }
  throw new Error(`file ${file} did not reach expected content; last text: ${JSON.stringify(last)}`);
}
