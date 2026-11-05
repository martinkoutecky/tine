import { node } from "../document";
import { clearOnBindingInvalidated } from "../binding";

type FindSheet = { host: () => HTMLElement | undefined; reveal: (id: string) => void };
const sheets = new Set<FindSheet>();
clearOnBindingInvalidated(() => sheets.clear());

/** Reuse a sheet's bounded render window without selecting a cell or ending edit. */
export function registerFindSheet(host: FindSheet["host"], reveal: FindSheet["reveal"]): () => void {
  const sheet = { host, reveal };
  sheets.add(sheet);
  return () => { sheets.delete(sheet); };
}

export function revealFindSheetBlock(id: string, scope: Element): void {
  for (const sheet of sheets) {
    const host = sheet.host();
    if (host && scope.contains(host)) sheet.reveal(id);
  }
}

/** The direct child containing a searched descendant, if this is its sheet. */
export function sheetFindChild(id: string, owner: string): string | undefined {
  for (let current = node(id); current?.parent; current = node(current.parent)) {
    if (current.parent === owner) return current.id;
  }
}
