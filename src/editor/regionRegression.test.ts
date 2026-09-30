import { beforeAll, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { loadSingle, resetStore } from "../document/workingSet";
import { doc } from "../document/model";
import { setSchedule, readSchedule } from "../document/edits/properties";
import { codeBodyProjection, codeBodyJoin } from "./codeFence";
beforeAll(() => initParser());
it("schedule edit preserves a leading literal wrapper", () => {
  resetStore();
  const raw = "```\nSCHEDULED: <2026-09-29 Tue>\n```";
  loadSingle({name:"P", title:"P", kind:"page", format:"md", pre_block:null, blocks:[{id:"b",raw,collapsed:false,children:[]}]});
  expect(readSchedule("b","scheduled")).toBeNull();
  setSchedule("b","scheduled",{y:2026,m:9,d:1});
  expect(doc.byId.b.raw).toContain(raw);
});
it("typing into an empty code card preserves the closer separator", () => {
  for (const [raw,format] of [["```js\n```","md"],["#+BEGIN_SRC js\n#+END_SRC","org"]] as const) {
    const p = codeBodyProjection(raw,format)!;
    const next = codeBodyJoin(p,"x");
    expect(codeBodyProjection(next,format)?.body).toBe("x");
  }
});
