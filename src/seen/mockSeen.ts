// The browser mock's seen-baseline store (ADR 0073): in memory, per graph and
// page key, so dev runs and screenshots exercise the real frontend path.
import type { SeenBaselineRequest } from "../backendTypes";

export function mockSeenBaseline(): (request: SeenBaselineRequest) => Promise<string[] | null> {
  const records = new Map<string, string[]>();
  return async (request) => {
    const key = `${request.graph}\n${request.page}`;
    if (request.op === "mark") records.set(key, [...new Set(request.hashes)].sort());
    else if (request.op === "forget") records.delete(key);
    else return records.get(key)?.slice() ?? null;
    return null;
  };
}
