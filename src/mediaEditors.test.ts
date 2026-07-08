import { describe, it, expect } from "vitest";
import { MEDIA_EDITORS, mediaEditorForAsset } from "./mediaEditors";
import { assetVersion, refreshAsset } from "./assetCache";

describe("media-editor registry (GH #38)", () => {
  it("matches drawio assets and not plain images", () => {
    const ed = mediaEditorForAsset("flow.drawio.svg");
    expect(ed?.id).toBe("drawio");
    expect(mediaEditorForAsset("../assets/a.drawio.svg")?.id).toBe("drawio");
    // A plain SVG/PNG is NOT a diagram-editor asset.
    expect(mediaEditorForAsset("foo.svg")).toBeUndefined();
    expect(mediaEditorForAsset("photo.png")).toBeUndefined();
    expect(mediaEditorForAsset("notdrawio.svgx")).toBeUndefined();
    expect(mediaEditorForAsset(null)).toBeUndefined();
    expect(mediaEditorForAsset("")).toBeUndefined();
  });

  it("matches excalidraw's svg/png (and bare) forms", () => {
    expect(mediaEditorForAsset("s.excalidraw.svg")?.id).toBe("excalidraw");
    expect(mediaEditorForAsset("s.excalidraw.png")?.id).toBe("excalidraw");
    expect(mediaEditorForAsset("s.excalidraw")?.id).toBe("excalidraw");
    expect(mediaEditorForAsset("s.excalidrawX")).toBeUndefined();
  });

  it("drawio has a create-blank template that is an editable SVG; excalidraw does not", () => {
    const drawio = MEDIA_EDITORS.find((e) => e.id === "drawio")!;
    expect(drawio.blank?.ext).toBe("drawio.svg");
    const svg = drawio.blank!.contents();
    // Renders as an SVG, carries the editable mxfile in the root `content` attr
    // (escaped), with the two base cells — i.e. an empty editable canvas.
    expect(svg.startsWith("<svg")).toBe(true);
    expect(svg).toContain("content=");
    expect(svg).toContain("mxGraphModel");
    expect(svg).toContain("&lt;mxfile"); // escaped, not raw markup in the body
    expect(MEDIA_EDITORS.find((e) => e.id === "excalidraw")!.blank).toBeUndefined();
  });
});

describe("asset refresh version signal (GH #38)", () => {
  it("bumps the per-asset version on refreshAsset", () => {
    const rel = "x/only-this-test.drawio.svg";
    const before = assetVersion(rel);
    refreshAsset(rel);
    expect(assetVersion(rel)).toBe(before + 1);
    refreshAsset(rel);
    expect(assetVersion(rel)).toBe(before + 2);
    // An unrelated asset is unaffected.
    expect(assetVersion("y/other.drawio.svg")).toBe(0);
  });
});
