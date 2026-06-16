// Task-marker cycling, matching OG Logseq's cycle-marker-state:
//   TODO -> DOING -> DONE -> (none)
//   LATER -> NOW -> DONE -> (none)
//   (none) -> LATER (":now" workflow) or TODO (":todo" workflow)
// Bound to mod+enter (Ctrl+Enter on Linux/Windows) in the editor, like OG.

export type Workflow = "now" | "todo";

const MARKERS = ["NOW", "LATER", "TODO", "DOING", "IN-PROGRESS", "DONE", "WAITING", "WAIT", "CANCELED", "CANCELLED"];

export function nextMarker(marker: string | null, workflow: Workflow): string | null {
  switch (marker) {
    case "TODO":
      return "DOING";
    case "DOING":
      return "DONE";
    case "LATER":
      return "NOW";
    case "NOW":
      return "DONE";
    case "DONE":
      return null;
    default:
      return workflow === "now" ? "LATER" : "TODO";
  }
}

/** Leading task marker of a block's first line, if any. */
export function leadingMarker(raw: string): string | null {
  const first = raw.split("\n", 1)[0];
  for (const m of MARKERS) {
    if (first === m || first.startsWith(m + " ")) return m;
  }
  return null;
}

/** Cycle the marker on `raw`; returns the new raw and the caret delta (change
 *  in length of the leading marker, so the editor can keep the caret put). */
export function cycleMarker(raw: string, workflow: Workflow): { raw: string; delta: number } {
  const lines = raw.split("\n");
  const first = lines[0];
  const cur = leadingMarker(raw);
  const next = nextMarker(cur, workflow);

  // strip current marker prefix
  let rest = first;
  if (cur) rest = first.slice(cur.length).replace(/^ /, "");
  const oldPrefixLen = first.length - rest.length;

  const newFirst = next ? `${next} ${rest}` : rest;
  const newPrefixLen = newFirst.length - rest.length;

  lines[0] = newFirst;
  return { raw: lines.join("\n"), delta: newPrefixLen - oldPrefixLen };
}
