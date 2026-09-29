// Task-marker cycling, matching OG Logseq's cycle-marker-state:
//   TODO -> DOING -> DONE -> (none)
//   LATER -> NOW -> DONE -> (none)
//   (none) -> LATER (":now" workflow) or TODO (":todo" workflow)
// Bound to mod+enter (Ctrl+Enter on Linux/Windows) in the editor, like OG.

import { matchLeadingMarker } from "../markers";

export type Workflow = "now" | "todo";

/** Re-export the one shared recognizer (src/markers.ts) so existing importers
 *  keep working. DUP-7: a second copy here had drifted from lsdoc/the renderer. */
export { leadingMarker } from "../markers";

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

/** Cycle the marker on `raw`; returns the new raw and the caret delta (change
 *  in length of the leading marker, so the editor can keep the caret put). The
 *  marker is spliced at its exact recognized offsets, so leading whitespace /
 *  blank lines it follows are preserved rather than gaining a second marker. */
export function cycleMarker(raw: string, workflow: Workflow): { raw: string; delta: number } {
  const cur = matchLeadingMarker(raw);
  const next = nextMarker(cur?.marker ?? null, workflow);

  // Text after the marker (one separating space stripped), as before — but taken
  // from the recognized offsets so an indented marker also desugars to one splice.
  let rest = raw;
  if (cur) rest = raw.slice(cur.end).replace(/^ /, "");
  const oldPrefixLen = raw.length - rest.length;

  const head = cur ? raw.slice(0, cur.start) : "";
  const newRaw = next ? `${head}${next} ${rest}` : `${head}${rest}`;
  const newPrefixLen = newRaw.length - rest.length;

  return { raw: newRaw, delta: newPrefixLen - oldPrefixLen };
}

/** Set the leading task marker explicitly, using the same recognized-offset
 *  splice as cycleMarker. `null` removes any existing marker. */
export function setMarker(raw: string, marker: string | null): string {
  const cur = matchLeadingMarker(raw);
  let rest = raw;
  if (cur) rest = raw.slice(cur.end).replace(/^ /, "");
  const head = cur ? raw.slice(0, cur.start) : "";
  return marker ? (rest ? `${head}${marker} ${rest}` : `${head}${marker}`) : `${head}${rest}`;
}
