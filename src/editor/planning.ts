// Planning normalization uses the shared native/wasm region editor.
import { editBlock } from "../render/parse";
import type { Format } from "../render/ast";
export function normalizePlanning(visible: string, format: Format): string {
  return editBlock(visible,format,{kind:"normalize_planning"});
}
