import { describe, expect, it } from "vitest";
import { callsImported, declaresArrayConstant, declaresFunction, importsFrom } from "./sourceOwnership";

// Necessity for the ownership helpers the architecture guards stand on: each
// accepts the equivalent spellings a text scan rejects, and rejects the
// violation a text scan would let through.
const F = "src/components/x.tsx";

describe("source ownership helpers", () => {
  it("importsFrom resolves aliases, sibling names and relative paths; ignores comments", () => {
    expect(importsFrom(F, "isLeafLike", "src/editor/queryIr", `import { isLeafLike } from "../editor/queryIr";`)).toBe(true);
    expect(importsFrom(F, "isLeafLike", "src/editor/queryIr", `import { other, isLeafLike as leaf } from "../editor/queryIr.ts";`)).toBe(true);
    expect(importsFrom(F, "default", "fixtures/p.json", `import policy from "../../fixtures/p.json";`)).toBe(true);
    expect(importsFrom(F, "isLeafLike", "src/editor/queryIr", `import { isLeafLike } from "../editor/otherIr";`)).toBe(false);
    expect(importsFrom(F, "isLeafLike", "src/editor/queryIr", `// import { isLeafLike } from "../editor/queryIr";`)).toBe(false);
  });
  it("declaresFunction sees declarations and function-valued constants only", () => {
    expect(declaresFunction(F, "urlDest", `function urlDest(x: string) { return x; }`)).toBe(true);
    expect(declaresFunction(F, "urlDest", `const urlDest = (x: string) => x;`)).toBe(true);
    expect(declaresFunction(F, "urlDest", `const urlDest = 3; // function urlDest() {}`)).toBe(false);
    expect(declaresFunction(F, "urlDest", `import { urlDest } from "./urlDest"; urlDest("a");`)).toBe(false);
  });
  it("declaresArrayConstant matches array-literal initializers by name", () => {
    expect(declaresArrayConstant(F, /^RAW_HTML_TAGS$/, `const RAW_HTML_TAGS = ["a"];`)).toBe(true);
    expect(declaresArrayConstant(F, /^RAW_HTML_TAGS$/, `const RAW_HTML_TAGS = policy.tags;`)).toBe(false);
  });
  it("callsImported follows the import under any alias and honours `within`", () => {
    const src = `import { soleBlockMacro as sole } from "../render/parse";
      export function detectMacro(raw: string) { return sole(raw, "md"); }
      export function other() { return 1; }`;
    expect(callsImported(F, "soleBlockMacro", "src/render/parse", undefined, src)).toBe(true);
    expect(callsImported(F, "soleBlockMacro", "src/render/parse", "detectMacro", src)).toBe(true);
    expect(callsImported(F, "soleBlockMacro", "src/render/parse", "other", src)).toBe(false);
    expect(callsImported(F, "soleBlockMacro", "src/render/other", undefined, src)).toBe(false);
    expect(callsImported(F, "soleBlockMacro", "src/render/parse", undefined, `import { soleBlockMacro } from "../render/parse";`)).toBe(false);
  });
});
