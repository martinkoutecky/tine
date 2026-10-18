export type TraceEvent = {
  kind: "fsync" | "rename" | "unlink";
  path: string;
  to?: string;
  ok: boolean;
};
export function parseTrace(text: string): TraceEvent[];
export function check(events: TraceEvent[]): string[];
