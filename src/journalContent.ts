// The journal-template guard (graph.ts ensureJournalTemplateForDay). It answers a
// different question from tine-store's `doc_has_content` (which journal days the
// calendar and carry treat as written): a template may replace a journal only when
// the user has written nothing in it at all. Port of master's `blockTreeHasText`
// (GH #550). OG goes further and fills only an absent or blank file.
interface ContentBlock { raw: string; children: readonly ContentBlock[] }

/** True when any block in this page, including descendants, holds any
 * non-whitespace text. Deliberately format- and grammar-free: Org prose, a
 * property-shaped line and a root-level `id::` are all the user's (OG-C5
 * L12-S1), and the template replaces every root. Cost O(blocks + text of one
 * page), iterative so nesting depth cannot overflow the stack; pure. */
export function journalHasContent(blocks: readonly ContentBlock[]): boolean {
  const pending = [...blocks];
  while (pending.length) {
    const block = pending.pop()!;
    if (block.raw.trim() !== "") return true;
    pending.push(...block.children);
  }
  return false;
}
