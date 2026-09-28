export interface PdfOutlineItem {
  id: string;
  label: string;
  destination: string | unknown[] | null;
  children: PdfOutlineItem[];
}

export const PDF_OUTLINE_MAX_NODES = 10_000;
export const PDF_OUTLINE_MAX_SCANNED_SLOTS = 100_000;

/** Convert array-shaped outline input into a UI tree. Emits at most 10,000 nodes,
 * reads at most 100,000 source slots (valid or not), and descends at most 63
 * levels; malformed entries are skipped, blank titles become "Untitled", and IDs
 * encode source positions. It does not cap title/destination size, and it retains destination arrays by
 * reference without validating their elements. Cycles terminate at the depth
 * cap; hostile getters or proxies may throw. No I/O; work is proportional to
 * scanned input slots plus emitted nodes, both capped. */
export function sanitizeOutlineItems(value: unknown): PdfOutlineItem[] {
  if (!Array.isArray(value)) return [];
  const sanitized: PdfOutlineItem[] = [];
  const pending = [{ source: value, target: sanitized, parentId: "outline", depth: 0 }];
  let count = 0;
  let scanned = 0;
  const open = () => count < PDF_OUTLINE_MAX_NODES && scanned < PDF_OUTLINE_MAX_SCANNED_SLOTS;
  while (pending.length && open()) {
    const { source, target, parentId, depth } = pending.pop()!;
    for (let index = 0; index < source.length && open(); index++) {
      scanned++;
      const candidate = source[index];
      if (!candidate || typeof candidate !== "object") continue;
      const raw = candidate as Record<string, unknown>;
      const id = `${parentId}-${index}`;
      const item: PdfOutlineItem = {
        id,
        label: typeof raw.title === "string" && raw.title.trim() ? raw.title : "Untitled",
        destination: typeof raw.dest === "string" || Array.isArray(raw.dest) ? raw.dest : null,
        children: [],
      };
      target.push(item);
      count++;
      if (depth < 63 && Array.isArray(raw.items)) {
        pending.push({ source: raw.items, target: item.children, parentId: id, depth: depth + 1 });
      }
    }
  }
  return sanitized;
}
