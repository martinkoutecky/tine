import type { Backend } from "./backend";

type JournalFileMethods = "listJournalConflicts" | "readJournalFile" | "listJournalFilenameMigrations";

/** The mock backend's journal-file surface (host state): duplicate days,
 *  title-named files and their proposed date names (demoed via `?conflicts`,
 *  whose journal files are fixture files). Trashing a file and migrating names
 *  are store write paths: the mock refuses them (ADR 0081). */
export function mockJournalFiles(text: (path: string) => Promise<string | undefined>): Pick<Backend, JournalFileMethods> {
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
    async readJournalFile(name: string): Promise<string> {
      const source = await text(`journals/${name}`);
      if (source === undefined) throw new Error(`journal file ${name} is not in the browser preview graph`);
      return source;
    },
    async listJournalFilenameMigrations() {
      // Not the conflicted 26th: the backend never proposes a duplicate day.
      return demo() ? [{ from: "Thursday, 25-06-2026.org", to: "2026_06_25.org" }] : [];
    },
  };
}
