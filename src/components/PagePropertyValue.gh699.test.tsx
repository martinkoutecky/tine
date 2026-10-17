import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { initParser } from "../render/parse";
import { PagePropertyValue } from "./PagePropertyValue";
import { PropertyRows } from "../render/PropertyRows";

beforeAll(initParser);
afterEach(() => { document.body.innerHTML = ""; });

for (const propertyKey of ["tags", "alias", "aliases"]) {
  for (const [value, selector, text] of [
    ["`literal`", "code", "literal"],
    ["**plain**", "strong", "plain"],
    ["[site](https://example.com)", 'a[href="https://example.com"]', "site"],
  ]) {
    it(`${propertyKey}: empty extraction preserves inline ${value} on pages and block rows`, () => {
      const host = document.createElement("div");
      const [current, setCurrent] = createSignal("#a #b");
      const dispose = render(() => <>
        <div class="page-value"><PagePropertyValue propertyKey={propertyKey} value={current()} format="md" /></div>
        <PropertyRows entries={[[propertyKey, current()]]} format="md" />
      </>, host);
      setCurrent(value);
      for (const scope of [".page-value", ".prop-value"]) {
        expect(host.querySelector(`${scope} ${selector}`)?.textContent).toBe(text);
        expect(host.querySelectorAll(`${scope} a.page-ref`)).toHaveLength(0);
      }
      setCurrent("#again");
      expect([...host.querySelectorAll(".page-value a.page-ref, .prop-value a.page-ref")].map(a => a.textContent)).toEqual(["again", "again"]);
      dispose();
    });
  }
}

for (const format of ["md", "org"] as const) {
  for (const value of ["#eng #exec #founder", "#eng,#exec,#founder", "#eng, #exec, #founder", "[[eng]] [[exec]] [[founder]]"]) {
    it(`${format}: property tokens navigate separately: ${value}`, () => {
      const host = document.createElement("div");
      const dispose = render(() => <PagePropertyValue propertyKey="tags" value={value} format={format} />, host);
      expect([...host.querySelectorAll("a.page-ref")].map(a => a.textContent)).toEqual([
        "eng", "exec", "founder",
      ]);
      dispose();
    });
    it(`${format}: block property rows use the same token names: ${value}`, () => {
      const host = document.createElement("div");
      const dispose = render(() => <PropertyRows entries={[["tags", value]]} format={format} />, host);
      expect([...host.querySelectorAll(".prop-value a.page-ref")].map(a => a.textContent)).toEqual(["eng", "exec", "founder"]);
      dispose();
    });
  }
  it(`${format}: mixed syntax retains plain and multi-word reference boundaries after a live update`, () => {
    const host = document.createElement("div");
    const [value, setValue] = createSignal("#a plain text #b");
    const dispose = render(() => <PagePropertyValue propertyKey="tags" value={value()} format={format} />, host);
    expect([...host.querySelectorAll("a.page-ref")].map(a => a.textContent).sort()).toEqual(["a", "b", "plain text"]);
    setValue("#[[multi word]] [[other ref]]");
    expect([...host.querySelectorAll("a.page-ref")].map(a => a.textContent)).toEqual(["multi word", "other ref"]);
    dispose();
  });
  it(`${format}: quoted values stay literal after live transitions`, () => {
    const host = document.createElement("div");
    const [value, setValue] = createSignal("#a #b");
    const dispose = render(() => <PagePropertyValue propertyKey="tags" value={value()} format={format} />, host);
    setValue('"#a [[b]]"');
    expect(host.querySelectorAll("a")).toHaveLength(0);
    expect(host.textContent).toBe('"#a [[b]]"');
    dispose();
  });
  it(`${format}: custom properties preserve mixed text and link only explicit references`, () => {
    const host = document.createElement("div");
    const dispose = render(() => <PropertyRows entries={[["related", "#a plain text #[[multi word]]"]]} format={format} />, host);
    expect([...host.querySelectorAll(".prop-value a.tag")].map(a => a.textContent?.replace(/^#/, ""))).toEqual(["a", "multi word"]);
    expect(host.querySelector(".prop-value")?.textContent).toContain("plain text");
    dispose();
  });
}
