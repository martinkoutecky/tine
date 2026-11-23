// Block ids of the browser mock's fixture pages (ADR 0081), for scripts that
// drive the dev server. The browser core gives a block a positional id (its
// file and sibling path), not a name the fixture chose, so a script names a
// block by its text and asks the core for the id, as the window reads it.

/** For each graph-relative path, a map from block raw text to the block id the
 * core answers. Runs in the page, through the app's own `backend()` (the dev
 * server serves the same module the app imports). Throws if a page is absent
 * or repeats a text, so a fixture that drifted from the script fails loudly. */
export function mockBlockIds(page, paths) {
  return page.evaluate(async (paths) => {
    const { backend } = await import("/src/backend.ts");
    const result = {};
    for (const path of paths) {
      const read = await backend().getPageByPath(path);
      if (!read) throw new Error(`the mock graph has no ${path}: is its URL flag set?`);
      const ids = result[path] = {};
      const walk = (blocks) => {
        for (const block of blocks) {
          if (block.raw in ids) throw new Error(`${path} repeats the block text ${JSON.stringify(block.raw)}`);
          ids[block.raw] = block.id;
          walk(block.children);
        }
      };
      walk(read.blocks);
    }
    return result;
  }, paths);
}

/** The id of the block whose text is `raw` in `ids` (one path's map), or a throw naming the text. */
export function idOf(ids, raw) {
  const id = ids[raw];
  if (!id) throw new Error(`no fixture block has the text ${JSON.stringify(raw)}`);
  return id;
}
