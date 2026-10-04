import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { initParser } from "../render/parse";

import { setDoc } from "../document/model";
import { loadSingle } from "../document/workingSet";
import { setGraphMeta } from "../graphSession";
import { resetStore } from "../document";
import { doc } from "../document/model";
import { resetTabsToJournals, route } from "../router";
beforeAll(initParser);
afterEach(() => { resetStore(); setGraphMeta(null); resetTabsToJournals(); document.body.replaceChildren(); vi.restoreAllMocks(); });
const raw = "what-is-this:: property before block content 1\nwhat-is-this:: property before block content 2\nclass:: test before\nblock properties test\nwhat-is-this:: property after block content 1\nwhat-is-this:: property after block content 2\nclass:: test after";
it("renders OG property rows and key navigation without changing raw content", async () => {
  setDoc({ loaded: true, feed: [], pages: [{ name: "Property parity", title: "Property parity", kind: "page", preBlock: null,
    roots: ["property-parity"], format: "md", readOnly: false, guide: false }],
    byId: { "property-parity": { id: "property-parity", raw, parent: null, page: "Property parity", children: [], collapsed: false } } });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <Block id="property-parity" />, host);
  try {
    const rows = host.querySelectorAll(".block-properties .prop");
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain("property before block content 2");
    expect(rows[1].textContent).toContain("test before");
    const link = rows[0].querySelector<HTMLAnchorElement>(".page-ref")!;
    expect(link).not.toBeNull();
    link.click();
    expect(route()).toMatchObject({ kind: "page", name: "what-is-this" });
    expect(doc.byId["property-parity"].raw).toBe(raw);
  } finally { dispose(); }
});

it.each([
  ["md", "body\nstatus:: last", "last"],
  ["org", "body\n:PROPERTIES:\n:status: first\n:status: last\n:END:", "last"],
] as const)("loaded %s properties use the same linked rows (including a sole trailing group)", (format, raw, expected) => {
  loadSingle({ name: "Loaded properties", title: "Loaded properties", kind: "page", format, pre_block: null,
    blocks: [{ id: "loaded-property", raw, children: [], collapsed: false }] });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <Block id="loaded-property" />, host);
  try {
    expect(host.querySelectorAll(".prop")).toHaveLength(1);
    expect(host.querySelector(".prop-key .page-ref")?.textContent).toBe("status");
    expect(host.querySelector(".prop-value")?.textContent).toBe(expected);
    expect(doc.byId["loaded-property"].raw).toBe(raw);
  } finally { dispose(); }
});
