import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { msg, renderEnglish } from "./i18n";
import { setToasts, toasts } from "./toasts";
import { maybeShowGitIgnoreNotice, resetGitIgnoreNoticeForTests } from "./gitIgnoreNotice";
import type { GitAutocommitStatus } from "./backendTypes";

const now = () => Promise.resolve();
const status = (last: Partial<NonNullable<GitAutocommitStatus["last"]>> | undefined) =>
  ({ last: last && { committed_at: null, commit: null, ...last } }) as unknown as GitAutocommitStatus;

beforeEach(() => { setToasts([]); resetGitIgnoreNoticeForTests(); });
afterEach(() => { vi.restoreAllMocks(); setToasts([]); });

describe("git .gitignore toast (D-20261026-GI, D-20261029-GT)", () => {
  it("toasts once, sticky, naming the file, when the backend added the rule", async () => {
    vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(status({ gitignore_added: "/g/.gitignore", gitignore_lines: [".syncthing.*"] }));
    await maybeShowGitIgnoreNotice("/g", now);
    await maybeShowGitIgnoreNotice("/g", now);
    expect(toasts()).toHaveLength(1);
    expect(toasts()[0].sticky).toBe(true);
    expect(toasts()[0].message).toBe(renderEnglish(msg("git-settings-ignore-added", { path: "/g/.gitignore", which: "syncthing" })));
  });

  it("waits for the worker's setup: a later status answer still toasts", async () => {
    const spy = vi.spyOn(backend(), "gitAutocommitStatus")
      .mockResolvedValueOnce(status(undefined))
      .mockResolvedValueOnce(status({}))
      .mockResolvedValue(status({ gitignore_added: "/g/.gitignore", gitignore_lines: [".syncthing.*"] }));
    await maybeShowGitIgnoreNotice("/g", now);
    expect(spy).toHaveBeenCalledTimes(3);
    expect(toasts()).toHaveLength(1);
  });

  it("says so when the write could not be confirmed", async () => {
    vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(status({ gitignore_failed: true, gitignore_lines: [".syncthing.*"] }));
    await maybeShowGitIgnoreNotice("/g", now);
    expect(toasts()[0].message).toBe(renderEnglish(msg("git-settings-ignore-failed", { which: "syncthing" })));
  });

  it("one combined toast when both lines were added at once, and the Tine line alone (D-20261029-GT)", async () => {
    vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValueOnce(
      status({ gitignore_added: "/g/.gitignore", gitignore_lines: [".syncthing.*", "logseq/.tine-*"] }),
    );
    await maybeShowGitIgnoreNotice("/g", now);
    expect(toasts()).toHaveLength(1);
    const both = toasts()[0].message;
    expect(both).toContain("Tine added .syncthing.* and logseq/.tine-* to /g/.gitignore");
    expect(both).toContain("removed from it, not from disk, in a separate Tine commit");
    expect(both).toContain("Conflict copies are still committed");

    setToasts([]);
    vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValueOnce(
      status({ gitignore_added: "/h/.gitignore", gitignore_lines: ["logseq/.tine-*"] }),
    );
    await maybeShowGitIgnoreNotice("/h", now);
    expect(toasts()).toHaveLength(1);
    expect(toasts()[0].message).toContain("Tine added logseq/.tine-* to /h/.gitignore");
    expect(toasts()[0].message).toContain("devices syncing one repository don't stop at a deleted page");
    expect(toasts()[0].message).not.toContain(".syncthing.*");

    setToasts([]);
    vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValueOnce(
      status({ gitignore_failed: true, gitignore_lines: [".syncthing.*", "logseq/.tine-*"] }),
    );
    await maybeShowGitIgnoreNotice("/i", now);
    expect(toasts()[0].message).toContain("couldn't confirm adding .syncthing.* and logseq/.tine-*");
  });

  it("shows nothing when nothing was added, or the status is unreadable", async () => {
    vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValueOnce(status({}));
    await maybeShowGitIgnoreNotice("/g", now);
    vi.spyOn(backend(), "gitAutocommitStatus").mockRejectedValue(new Error("boom"));
    await maybeShowGitIgnoreNotice("/h", now);
    expect(toasts()).toHaveLength(0);
  });
});
