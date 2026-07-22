import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { initLinkDefault, linkAutocompletePolicy, migrateLinkAutocompletePolicy, setLinkAutocompletePolicy } from "./linkDefault";

afterEach(() => vi.restoreAllMocks());

describe("link autocomplete policy migration", () => {
  it("keeps valid strings and maps the legacy boolean without treating Create-first as OG", () => {
    expect(migrateLinkAutocompletePolicy("adaptive", false)).toBe("adaptive");
    expect(migrateLinkAutocompletePolicy("existing", false)).toBe("existing");
    expect(migrateLinkAutocompletePolicy("typed", true)).toBe("typed");
    expect(migrateLinkAutocompletePolicy("invalid", true)).toBe("existing");
    expect(migrateLinkAutocompletePolicy(undefined, false)).toBe("adaptive");
    expect(migrateLinkAutocompletePolicy(undefined, undefined)).toBe("adaptive");
  });

  it("falls back to adaptive when the persisted-policy backend read fails", async () => {
    setLinkAutocompletePolicy("typed");
    vi.spyOn(backend(), "getAppString").mockRejectedValueOnce(new Error("settings unavailable"));
    await initLinkDefault();
    expect(linkAutocompletePolicy()).toBe("adaptive");
  });

  it("migrates an invalid/missing string through the retained legacy read and persists the result", async () => {
    const getString = vi.spyOn(backend(), "getAppString").mockResolvedValueOnce("not-a-policy");
    const getLegacy = vi.spyOn(backend(), "getLinkFirstMatch").mockResolvedValueOnce(true);
    const persist = vi.spyOn(backend(), "setAppString").mockResolvedValueOnce();
    await initLinkDefault();
    expect(getString).toHaveBeenCalledWith("link_autocomplete_policy", "");
    expect(getLegacy).toHaveBeenCalledOnce();
    expect(linkAutocompletePolicy()).toBe("existing");
    expect(persist).toHaveBeenCalledWith("link_autocomplete_policy", "existing");
  });

  it("keeps adaptive when both legacy migration reads fail", async () => {
    vi.spyOn(backend(), "getAppString").mockResolvedValueOnce("");
    vi.spyOn(backend(), "getLinkFirstMatch").mockRejectedValueOnce(new Error("legacy unavailable"));
    await initLinkDefault();
    expect(linkAutocompletePolicy()).toBe("adaptive");
  });

  it("keeps the newest overlapping refresh when the older backend read finishes last", async () => {
    let resolveOlder!: (value: string) => void;
    const older = new Promise<string>((resolve) => { resolveOlder = resolve; });
    const getString = vi.spyOn(backend(), "getAppString")
      .mockImplementationOnce(() => older)
      .mockResolvedValueOnce("typed");
    const first = initLinkDefault();
    const second = initLinkDefault();
    await second;
    expect(linkAutocompletePolicy()).toBe("typed");
    resolveOlder("existing");
    await first;
    expect(getString).toHaveBeenCalledTimes(2);
    expect(linkAutocompletePolicy()).toBe("typed");
  });
});
