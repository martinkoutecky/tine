import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { msg, renderEnglish } from "./i18n";
import { setToasts, toasts } from "./toasts";
import { maybeShowSyncthingIgnoreNotice, resetSyncthingIgnoreNoticeForTests } from "./gitIgnoreNotice";
import type { GitAutocommitStatus } from "./backendTypes";

const now = () => Promise.resolve();
const status = (last: Partial<NonNullable<GitAutocommitStatus["last"]>> | undefined) =>
  ({ last: last && { committed_at: null, commit: null, ...last } }) as unknown as GitAutocommitStatus;

beforeEach(() => { setToasts([]); resetSyncthingIgnoreNoticeForTests(); });
afterEach(() => { vi.restoreAllMocks(); setToasts([]); });

describe("Syncthing .gitignore toast (D-20261026-GI)", () => {
  it("toasts once, sticky, naming the file, when the backend added the rule", async () => {
    vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(status({ syncthing_ignore_added: "/g/.gitignore" }));
    await maybeShowSyncthingIgnoreNotice("/g", now);
    await maybeShowSyncthingIgnoreNotice("/g", now);
    expect(toasts()).toHaveLength(1);
    expect(toasts()[0].sticky).toBe(true);
    expect(toasts()[0].message).toBe(renderEnglish(msg("git-settings-syncthing-ignore-added", { path: "/g/.gitignore" })));
  });

  it("waits for the worker's setup: a later status answer still toasts", async () => {
    const spy = vi.spyOn(backend(), "gitAutocommitStatus")
      .mockResolvedValueOnce(status(undefined))
      .mockResolvedValueOnce(status({}))
      .mockResolvedValue(status({ syncthing_ignore_added: "/g/.gitignore" }));
    await maybeShowSyncthingIgnoreNotice("/g", now);
    expect(spy).toHaveBeenCalledTimes(3);
    expect(toasts()).toHaveLength(1);
  });

  it("says so when the write could not be confirmed", async () => {
    vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(status({ syncthing_ignore_failed: true }));
    await maybeShowSyncthingIgnoreNotice("/g", now);
    expect(toasts()[0].message).toBe(renderEnglish(msg("git-settings-syncthing-ignore-failed")));
  });

  it("shows nothing when nothing was added, or the status is unreadable", async () => {
    vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValueOnce(status({}));
    await maybeShowSyncthingIgnoreNotice("/g", now);
    vi.spyOn(backend(), "gitAutocommitStatus").mockRejectedValue(new Error("boom"));
    await maybeShowSyncthingIgnoreNotice("/h", now);
    expect(toasts()).toHaveLength(0);
  });
});
