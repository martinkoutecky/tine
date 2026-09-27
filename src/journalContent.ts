// Shared fixture with tine-store::model::doc_has_content:
// tests/fixtures/journal-content.json. Keep this DTO-side predicate aligned
// with Rust because both decide whether a journal template may replace a page.
interface ContentBlock { raw: string; children: readonly ContentBlock[] }

/** True when any block in this page contains a non-property line, including
 * descendants. Cost: O(blocks and text of one page). Pure; no I/O or failure
 * fallback. Callers do not need to know the nesting or property-line grammar.
 * Keep the shared fixture aligned with tine-store::model::doc_has_content. */
export function journalHasContent(blocks: readonly ContentBlock[]): boolean {
  const pending = [...blocks];
  while (pending.length) {
    const block = pending.pop()!;
    if (block.raw.split("\n").some((line: string) => {
      const trimmed = line.trim();
      return trimmed !== "" && !/^[A-Za-z0-9_./-]+\s*::/.test(trimmed);
    })) return true;
    pending.push(...block.children);
  }
  return false;
}
