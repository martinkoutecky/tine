/** Bound untrusted PDF outline metadata before recursive UI rendering. */
export interface PdfOutlineItem {
  id: string;
  label: string;
  destination: string | unknown[] | null;
  children: PdfOutlineItem[];
}

export function sanitizeOutlineItems(value: unknown): PdfOutlineItem[] {
  if (!Array.isArray(value)) return [];
  const sanitized: PdfOutlineItem[] = [];
  const pending = [{ source: value, target: sanitized, parentId: "outline", depth: 0 }];
  let count = 0;
  while (pending.length && count < 10000) {
    const { source, target, parentId, depth } = pending.pop()!;
    for (let index = 0; index < source.length && count < 10000; index++) {
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
