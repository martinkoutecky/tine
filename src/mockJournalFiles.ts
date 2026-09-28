import type { Backend } from "./backend";

type JournalFileMethods = "listJournalConflicts" | "trashJournalFile" | "readJournalFile"
  | "listJournalFilenameMigrations" | "applyJournalFilenameMigrations";

/** The mock backend's journal-file surface: duplicate days, title-named files
 *  and their proposed date names (demoed via `?conflicts`). */
export function mockJournalFiles(): Pick<Backend, JournalFileMethods> {
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
    async trashJournalFile(): Promise<void> {
      // no-op in the browser mock
    },
    async readJournalFile(name: string): Promise<string> {
      return name.startsWith("Friday")
        ? "* something something\n*\n"
        : "* Tried out the Org demo graph in Tine today\n* TODO follow up on the [[kitchen-sink]] feature tour\nSCHEDULED: <2026-06-27 Sat>\n* DONE loaded the graph and clicked around\n";
    },
    async listJournalFilenameMigrations() {
      return demo() ? [{ from: "Friday, 26-06-2026.org", to: "2026_06_26.org" }] : [];
    },
    async applyJournalFilenameMigrations() {
      return { migrated: 0, skipped: [] };
    },
  };
}
