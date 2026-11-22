import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
const source = (path: string) => readFileSync(path, "utf8");
const rule = "I-12: reuse the shared answerer; exemplars: inline.tsx::assetRelPath, tine-core/src/media_mime.rs, pageIdentityKey and blockRegions.id";

it(rule + " for asset paths and MIME", () => {
  const overlay = source("src/components/AudioOverlay.tsx");
  expect(overlay, rule).not.toContain("function relOf");
  for (const path of ["src/assetCache.ts", "src/render/inline.tsx", "src/components/AudioOverlay.tsx"]) {
    const code = source(path);
    expect(code, `${rule}; ${path}`).toContain("mime_from_path");
    expect(code, `${rule}; ${path}`).not.toMatch(/["'](?:audio|video|image)\/[a-z+.-]+["']/);
  }
  const native = source("src-tauri/src/media_protocol.rs");
  expect(native, rule).not.toContain("fn mime(");
  expect(native, rule).toContain("media_mime::from_path");
  expect(source("crates/lsdoc-wasm/src/lib.rs"), rule).toContain('tine_core::media_mime::from_path(path)');
});

it(rule + " for permalink identity", () => {
  const code = source("src/publishedPermalink.ts");
  expect(code, rule).toContain("pageLink(target.page)");
  expect(code, rule).toContain("reads.resolveBlock(target.block)");
  expect(code, rule).not.toMatch(/blockRegions|identityFold|raw\.includes|toLowerCase\(\)/);
});

it("I-4/I-12: export uses the parser-owned visible body; exemplar: render/parse.ts::editBlock", () => {
  for (const path of ["src/editor/exportMarkup.ts", "src/editor/exportProjection.ts"]) {
    const code = source(path);
    expect(code).toContain("editBlock");
    expect(code).not.toMatch(/isPropertyLine|orgBlockDrawerRange/);
  }
  expect(source("src/editor/exportText.ts")).toContain("exportSourceRaw(n, opts.removeProperties)");
});
