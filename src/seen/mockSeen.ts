// The browser mock's seen-baseline store (ADR 0073): in memory, per graph and
// page key, so dev runs and screenshots exercise the real frontend path.
import type { SeenBaselineWrite } from "../backendTypes";

export function mockSeenBaseline() {
  const records = new Map<string, string[]>();
  return {
    async readSeenBaseline(graph: string, page: string): Promise<string[] | null> {
      return records.get(`${graph}\n${page}`)?.slice() ?? null;
    },
    async writeSeenBaseline(request: SeenBaselineWrite): Promise<void> {
      const key = `${request.graph}\n${request.page}`;
      if (request.op === "mark") records.set(key, [...new Set(request.hashes)].sort());
      else records.delete(key);
    },
  };
}
