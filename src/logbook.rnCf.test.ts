import { expect, it } from "vitest";
import { applyMarkerTransition } from "./logbook";

for (const format of ["md", "org"] as const) {
  for (const seconds of [false, true]) {
    it(`keeps and closes a bare running task's clock (${format}, seconds=${seconds})`, () => {
      const start = seconds ? "2026-01-01 Thu 09:00:00" : "2026-01-01 Thu 09:00";
      const drawer = `\n:LOGBOOK:\nCLOCK: [${start}]\n:END:`;
      for (const marker of ["DOING", "NOW"]) {
        const old = marker + drawer;
        expect(applyMarkerTransition(old, `${marker} work${drawer}`, format, true, seconds))
          .toBe(`${marker} work${drawer}`);
        const done = applyMarkerTransition(old, `DONE work${drawer}`, format, true, seconds);
        expect(done.match(/CLOCK:/g)).toHaveLength(1);
        expect(done).toContain(`CLOCK: [${start}]--[`);
      }
    });
  }
}
