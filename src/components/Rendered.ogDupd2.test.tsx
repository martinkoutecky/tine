import { expect, it } from "vitest";
import { detectMacro } from "./Rendered";
it("D15 only specializes one parser-visible macro", () => {
  expect(detectMacro("{{query (task TODO)}}\nfoo::bar")).toBeNull();
  expect(detectMacro("{{embed [[A]]}} {{embed [[B]]}}")).toBeNull();
  expect(detectMacro("{{embed [[A]]}}\nklíč:: hodnota")?.kind).toBe("embed");
  expect(detectMacro("{{embed [[A]]}}\n:PROPERTIES:\n:klíč: hodnota\n:END:", "org")?.kind).toBe("embed");
});
