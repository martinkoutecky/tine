import type { Backend } from "./backend";

/** The mock backend's journal-file host state: the conflict demo's inventory
 *  of duplicate days (demoed via `?conflicts`, whose journal files are fixture
 *  files). A file's text is the core's answer (`read_journal_file_json`).
 *  Filename migration proposals, trashing a file and migrating names are the
 *  store's: the mock refuses them until the core exposes them (ADR 0081). */
export function mockJournalFiles(): Pick<Backend, "listJournalConflicts"> {
  const demo = () => typeof location !== "undefined" && /[?&]conflicts\b/.test(location.search);
  return {
    async listJournalConflicts() {
      // Default demo state is clean: no sticky "duplicate journal day" toast and no
      // reconcile banner cluttering the marketing screenshots. The reconcile flow is
      // demoed on demand via `?conflicts` (mirrors the `?big` virtualization gate).
      if (!demo()) return [];
      return [
        {
          title: "Friday, 26-06-2026",
          files: [
            { name: "2026_06_26.org", path: "journals/2026_06_26.org", preview: "Tried out the Org demo graph in Tine today", canonical: true },
            { name: "Friday, 26-06-2026.org", path: "journals/Friday, 26-06-2026.org", preview: "something something", canonical: false },
          ],
        },
      ];
    },
  };
}
