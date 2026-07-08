// Registry of external media editors (GH #38) — the one generic seam for the
// "render-as-image + edit-externally + refresh" workflow. Each entry pairs a
// filename matcher with a persisted command-template setting (Settings → Files),
// and optionally a blank template for the slash-command create-blank flow. drawio
// and Excalidraw are entries; a new tool is one more entry, nothing else changes.
//
// The asset itself is always an ordinary image reference (`![](../assets/x.…)`),
// rendered by the normal image pipeline (AssetImage) — Tine never parses the
// embedded diagram XML, and never inlines the SVG (only <img>). See the plan and
// docs/FEATURES.md.

export interface MediaEditor {
  /** Stable id — also the backend autodetect probe id (detect_media_editor). */
  id: string;
  /** Action-bar button label, e.g. "Edit in draw.io". */
  label: string;
  /** Settings-row label, e.g. "draw.io command". */
  settingLabel: string;
  /** Filenames this editor handles. LAST-extension aware (`x.drawio.svg`). */
  match: RegExp;
  /** app_string key holding the user's configured command template. */
  settingKey: string;
  /** Create-blank support for a slash command: the new asset's extension (after
   *  the stem, no leading dot) and the blank file's text contents. Omitted for
   *  export-from-elsewhere tools (Excalidraw has no in-app "new"). */
  blank?: { ext: string; contents: () => string };
  /** Show an Autodetect button (backed by detect_media_editor). */
  detectable: boolean;
}

// A blank drawio *editable* SVG: the diagram lives (uncompressed) in the root
// `content` attribute as an escaped mxfile with just the two base cells, so
// drawio reopens it as an empty editable canvas rather than importing a flat
// image. The visible SVG body is empty (blank canvas).
// NOTE (validate on a real drawio install — plan risk #1): if a build of drawio
// treats this as a flat import, fall back to seeding a native `.drawio` file.
function drawioBlankSvg(): string {
  const model =
    '<mxGraphModel dx="800" dy="600" grid="1" gridSize="10" guides="1" tooltips="1" ' +
    'connect="1" arrows="1" fold="1" page="1" pageScale="1" pageWidth="850" ' +
    'pageHeight="1100" math="0" shadow="0"><root><mxCell id="0"/>' +
    '<mxCell id="1" parent="0"/></root></mxGraphModel>';
  const mxfile = `<mxfile host="app.diagrams.net"><diagram id="blank" name="Page-1">${model}</diagram></mxfile>`;
  const content = mxfile
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
  return (
    '<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" ' +
    `version="1.1" width="1px" height="1px" viewBox="-0.5 -0.5 1 1" content="${content}">` +
    "<defs/><g/></svg>\n"
  );
}

export const MEDIA_EDITORS: MediaEditor[] = [
  {
    id: "drawio",
    label: "Edit in draw.io",
    settingLabel: "draw.io command",
    match: /\.drawio\.svg$/i,
    settingKey: "media_editor.drawio.command",
    blank: { ext: "drawio.svg", contents: drawioBlankSvg },
    detectable: true,
  },
  {
    id: "excalidraw",
    label: "Edit in Excalidraw",
    settingLabel: "Excalidraw command",
    // Excalidraw's editable exports: name.excalidraw.svg / .png (and bare .excalidraw
    // scene JSON, which renders as an attachment chip, not an image).
    match: /\.excalidraw(\.(svg|png))?$/i,
    settingKey: "media_editor.excalidraw.command",
    // No create-blank: Excalidraw is export-from-elsewhere (excalidraw.com / desktop),
    // so there is no `/excalidraw` new-file flow — only "Edit in Excalidraw".
    detectable: false,
  },
];

/** The editor (if any) that handles a given assets-relative filename. */
export function mediaEditorForAsset(rel: string | null | undefined): MediaEditor | undefined {
  if (!rel) return undefined;
  return MEDIA_EDITORS.find((e) => e.match.test(rel));
}
