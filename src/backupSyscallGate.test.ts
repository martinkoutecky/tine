// The launch-backup syscall gate's checker (scripts/check-backup-syscalls.mjs,
// REVIEW-backup-zip-2 R4/E4) against hand-built traces: a keeper witness
// counts only in the victim's own `<id>.zips/` and only while it is present.
import { describe, expect, it } from "vitest";
import { check, parseTrace, type TraceEvent } from "../scripts/check-backup-syscalls.mjs";

const Z = "/t/app/backups/g.zips";
const up = (p: string) => p.slice(0, p.lastIndexOf("/"));
const ev = (kind: TraceEvent["kind"], path: string, to?: string): TraceEvent => ({ kind, path, to, ok: true });
const marker = (name: string) => [ev("unlink", `/tine-syscall-gate-marker/${name}`)];
const chain = (zips: string) => [ev("fsync", zips), ev("fsync", up(zips)), ev("fsync", up(up(zips)))];
const publish = (zips: string, name: string) => [
  ev("fsync", `${zips}/.partial-${name}`),
  ev("rename", `${zips}/.partial-${name}`, `${zips}/${name}`),
  ...chain(zips),
];
const settle = (zip: string) => [ev("fsync", zip), ...chain(up(zip))];
const unlink = (zip: string) => [ev("unlink", zip)];

function trace(settingsPrune: TraceEvent[]): TraceEvent[] {
  return [
    ...marker("first-publication"),
    ...publish(Z, "a.zip"),
    ...marker("unchanged-settled"),
    ...settle(`${Z}/a.zip`),
    ...marker("changed-publication"),
    ...publish(Z, "b.zip"),
    ...marker("publication-with-prune"),
    ...publish(Z, "c.zip"),
    ...unlink(`${Z}/a.zip`),
    ...marker("publication-keep-12"),
    ...publish(Z, "d.zip"),
    ...marker("settings-prune"),
    ...settingsPrune,
    ...marker("end"),
  ];
}

describe("backup syscall gate checker", () => {
  it("accepts a trace whose every unlink follows a settled keeper in its own namespace", () => {
    expect(check(trace([...settle(`${Z}/d.zip`), ...unlink(`${Z}/b.zip`), ...unlink(`${Z}/c.zip`)]))).toEqual([]);
  });

  it("refuses a keeper certified in another graph's namespace", () => {
    const problems = check(
      trace([...settle("/other/app/backups/f.zips/keeper.zip"), ...unlink(`${Z}/b.zip`), ...unlink(`${Z}/c.zip`)]),
    );
    expect(problems.filter((p) => p.startsWith("settings-prune:"))).toHaveLength(2);
  });

  it("refuses a keeper that was itself unlinked before the victim", () => {
    // d and c each name the other as keeper; once d is gone, c's unlink
    // leaves no snapshot behind.
    const problems = check(
      trace([...settle(`${Z}/d.zip`), ...settle(`${Z}/c.zip`), ...unlink(`${Z}/d.zip`), ...unlink(`${Z}/c.zip`)]),
    );
    expect(problems).toEqual([
      "settings-prune: c.zip unlinked without a settled keeper still present in its own namespace",
    ]);
  });

  // Hosted CI 39083439899: another thread exited during the partial's fsync,
  // strace split the call, and the gate reported a missing fsync.
  it("rejoins a call that strace split around another thread's event", () => {
    const P = `${Z}/.partial-a.zip`;
    const events = parseTrace(
      [
        `100 fsync(6<${P}> <unfinished ...>`,
        "101 +++ exited with 0 +++",
        "100 <... fsync resumed>) = 0",
        `100 renameat2(AT_FDCWD</w>, "${P}", AT_FDCWD</w>, "${Z}/a.zip", RENAME_NOREPLACE <unfinished ...>`,
        `102 unlink("/elsewhere") = 0`,
        "100 <... renameat2 resumed>) = 0",
      ].join("\n"),
    );
    expect(events).toEqual([
      { kind: "fsync", path: P, ok: true },
      { kind: "unlink", path: "/elsewhere", ok: true },
      { kind: "rename", path: P, to: `${Z}/a.zip`, ok: true },
    ]);
  });
});
