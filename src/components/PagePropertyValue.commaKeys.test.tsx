// F12 (OG block.cljs `property-cp`, text.cljs `separated-by-commas?`): every comma item of a
// `:property/separated-by-commas` key is a page link, on a page header and in block rows.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { PagePropertyValue } from "./PagePropertyValue";
import { PropertyRows } from "../render/PropertyRows";
import { setGraphMeta } from "../graphSession";
import type { GraphMeta } from "../types";

beforeAll(initParser);
afterEach(() => { setGraphMeta(null); document.body.innerHTML = ""; });

const links = (host: HTMLElement) => [...host.querySelectorAll("a.page-ref")].map((a) => a.textContent);

describe("comma-separated property values", () => {
  it("links each item of a configured key, in the page header and in block rows", () => {
    setGraphMeta({ linkable_property_keys: ["alias", "aliases", "tags", "technique"] } as GraphMeta);
    const host = document.body.appendChild(document.createElement("div"));
    const dispose = render(() => <>
      <div class="page"><PagePropertyValue propertyKey="technique" value="Time synchronization, Continuous routing frame" format="md" /></div>
      <PropertyRows entries={[["Technique", "Alpha, [[Beta]]"]]} format="md" />
    </>, host);
    try {
      expect(links(host.querySelector(".page")!)).toEqual(["Time synchronization", "Continuous routing frame"]);
      expect(links(host.querySelector(".prop-value")!)).toEqual(["Alpha", "Beta"]);
    } finally { dispose(); }
  });
  it("leaves a key that is not listed as plain text", () => {
    setGraphMeta({ linkable_property_keys: ["alias", "aliases", "tags"] } as GraphMeta);
    const host = document.body.appendChild(document.createElement("div"));
    const dispose = render(() => <PagePropertyValue propertyKey="technique" value="Alpha, Beta" format="md" />, host);
    try { expect(links(host)).toEqual([]); expect(host.textContent).toBe("Alpha, Beta"); } finally { dispose(); }
  });
  it("a key the graph excludes from reference parsing is not linked even when built in (alias)", () => {
    setGraphMeta({ linkable_property_keys: ["aliases", "tags"] } as GraphMeta);
    const host = document.body.appendChild(document.createElement("div"));
    const dispose = render(() => <PagePropertyValue propertyKey="alias" value="Alpha, Beta" format="md" />, host);
    try { expect(links(host)).toEqual([]); } finally { dispose(); }
  });
  it("without the graph's answer only the built-ins link (exports, tests)", () => {
    const host = document.body.appendChild(document.createElement("div"));
    const dispose = render(() => <><PagePropertyValue propertyKey="tags" value="a, b" format="md" /><PagePropertyValue propertyKey="technique" value="c, d" format="md" /></>, host);
    try { expect(links(host)).toEqual(["a", "b"]); } finally { dispose(); }
  });
});
