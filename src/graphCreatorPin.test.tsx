import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { createNewGraph, ensureJournalTemplateForDay, loadGraphPath } from "./graph";
import { journalTitle } from "./journal";
import { flushPage, pageByName, resetStore, setRaw } from "./document";
import { loadSingle } from "./document/workingSet";
import { setGraphMeta } from "./graphSession";
import type { GraphMeta, PageDto, PageRead } from "./types";

const ROOT = "/tmp/creator-pin-graph";
const meta = (template: string | null): GraphMeta => ({
  root: ROOT, journals_dir: "journals", pages_dir: "pages", preferred_workflow: "now", shortcuts: {},
  start_of_week: 6, block_hidden_properties: [], linked_references_collapsed_threshold: 100, default_journal_template: template, favorites: [],
  journal_page_title_format: "MMM do, yyyy", journal_file_name_format: "yyyy_MM_dd", preferred_format: "md",
  macros: {}, enable_timetracking: true, show_brackets: true, logbook_with_second_support: true,
  logbook_enabled_in_timestamped_blocks: false, logbook_enabled_in_all_blocks: false, guide_announced: true, mobile_gestures_disabled_in_block_with_tags: [],
});
let path: string;

beforeEach(() => { resetStore(); setGraphMeta(null); path = `journals/${journalTitle(new Date())}.md`; });
afterEach(() => { vi.restoreAllMocks(); resetStore(); setGraphMeta(null); localStorage.clear(); });

describe("graph creators that bypass the save engine", () => {
  for (const [label, template] of [["journal template", "Daily"], ["demo seed", null]] as const) {
    it(`${label} creates today's journal file; a subsequent loaded edit saves against its revision`, async () => {
      const api = backend();
      const files = new Map<string, { dto: PageDto; rev: string }>();
      vi.spyOn(api, "inspectGraphAccess").mockResolvedValue({ graph_root: ROOT, external_assets_path: null, approved: true });
      vi.spyOn(api, "loadGraph").mockResolvedValue({ kind: "loaded", meta: meta(template), binding_generation: 1 });
      vi.spyOn(api, "getPage").mockResolvedValue(null);
      vi.spyOn(api, "resolvePage").mockResolvedValue({ kind: "absent", id: path });
      vi.spyOn(api, "listTemplates").mockResolvedValue([{ name: "Daily", page: "Templates", kind: "page", blocks: [{ id: "template", raw: "Template body", collapsed: false, children: [] }] }]);
      vi.spyOn(api, "readCustomCss").mockResolvedValue("");
      vi.spyOn(api, "pickFolder").mockResolvedValue("/tmp");
      vi.spyOn(api, "createGraph").mockResolvedValue(ROOT);
      const save = vi.spyOn(api, "savePages").mockImplementation(async (entries) => { const { id: id, page: dto } = entries[0];
        const rev = files.has(id) ? "edited-rev" : "created-rev";
        files.set(id, { dto: structuredClone(dto), rev });
        return { ok: [rev] };
      });
      const result = template ? await loadGraphPath(ROOT) : await createNewGraph();
      expect(result.kind).toBe("loaded");
      // The visible Journals surface materializes the template (master 5bb8ce020).
      if (template) expect(await ensureJournalTemplateForDay(new Date())).toBe("ready");
      expect(files.size).toBe(1);
      const [id, created] = [...files.entries()][0];
      expect(created.dto.kind).toBe("journal");
      expect(created.dto.blocks).toHaveLength(1);
      expect(created.dto.blocks[0].raw).toContain(template ? "Template body" : "today's journal");
      expect(save.mock.calls[0][0][0].baseRev).toBeNull();
      const loaded: PageRead = { ...created.dto, id, rev: created.rev };
      loadSingle(loaded);
      setRaw(pageByName(loaded.name)!.roots[0], "following edit");
      expect(await flushPage(loaded.name)).toBe(true);
      expect(save.mock.calls.at(-1)?.[0][0].baseRev).toBe("created-rev");
      expect(files.get(id)!.dto.blocks.map((b) => b.raw)).toEqual(["following edit"]);
    });
  }
});
