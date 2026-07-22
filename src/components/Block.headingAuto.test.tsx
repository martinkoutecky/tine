import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import {
  doc,
  indentBlock,
  loadSingle,
  outdentBlock,
  pageByName,
  pageToDto,
  resetStore,
  setHeading,
} from "../store";
import { endEdit } from "../editorController";
import type { BlockDto, Format, PageDto } from "../types";
import { Block } from "./Block";
import { RefBlocks } from "./RefBlocks";

beforeAll(() => initParser());

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

const auto = (id: string, children: BlockDto[] = []): BlockDto => ({
  id,
  raw: `${id}\nheading:: true`,
  collapsed: false,
  children,
  properties: [["heading", "true"]],
});

const plain = (id: string, children: BlockDto[] = []): BlockDto => ({
  id,
  raw: id,
  collapsed: false,
  children,
});

function chain(prefix: string, depth: number, leaf: BlockDto): BlockDto {
  let out = leaf;
  for (let level = depth - 1; level >= 0; level -= 1) out = plain(`${prefix}-${level}`, [out]);
  return out;
}

function page(blocks: BlockDto[], format: Format = "md"): PageDto {
  return { name: "Headings", kind: "page", title: "Headings", pre_block: null, blocks, format };
}

function mountMain(): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(
    () => <For each={pageByName("Headings")?.roots ?? []}>{(id) => <Block id={id} />}</For>,
    root,
  );
  return { root, dispose };
}

function headingClass(root: ParentNode, id: string): string {
  return root.querySelector(`[data-block-id="${id}"] .heading-text`)?.className ?? "";
}

describe("automatic heading effective-level invariant", () => {
  it.each([
    [0, "h1"],
    [1, "h2"],
    [4, "h5"],
    [7, "h6"],
  ] as const)("renders heading:: true at depth %i as %s", (depth, expected) => {
    const id = `auto-${depth}`;
    loadSingle(page([chain(`chain-${depth}`, depth, auto(id))]));
    const { root, dispose } = mountMain();
    try {
      expect(headingClass(root, id).split(/\s+/)).toContain(expected);
    } finally {
      dispose();
    }
  });

  it("lets an explicit numeric heading beat boolean auto state", () => {
    const explicit: BlockDto = {
      id: "explicit",
      raw: "### Explicit\nheading:: true",
      collapsed: false,
      children: [],
      heading_level: 3,
      properties: [["heading", "true"]],
    };
    loadSingle(page([plain("parent", [explicit])]));
    const { root, dispose } = mountMain();
    try {
      expect(headingClass(root, "explicit").split(/\s+/)).toContain("h3");
      expect(headingClass(root, "explicit").split(/\s+/)).not.toContain("h2");
    } finally {
      dispose();
    }
  });

  it("reacts to indent and outdent without persisting a computed level", () => {
    loadSingle(page([plain("previous"), auto("moving")]));
    const { root, dispose } = mountMain();
    try {
      expect(headingClass(root, "moving").split(/\s+/)).toContain("h1");
      indentBlock("moving", 0);
      endEdit("blur");
      expect(doc.byId.moving.raw).toBe("moving\nheading:: true");
      expect(headingClass(root, "moving").split(/\s+/)).toContain("h2");
      outdentBlock("moving", 0);
      endEdit("blur");
      expect(doc.byId.moving.raw).toBe("moving\nheading:: true");
      expect(headingClass(root, "moving").split(/\s+/)).toContain("h1");
    } finally {
      dispose();
    }
  });

  it("uses the same effective level in Block and RefBlocks", () => {
    const nested = plain("root", [auto("same")]);
    loadSingle(page([nested]));
    const main = mountMain();
    const refRoot = document.createElement("div");
    document.body.appendChild(refRoot);
    const disposeRef = render(() => <RefBlocks blocks={[nested]} page="Headings" pageKind="page" />, refRoot);
    try {
      expect(headingClass(main.root, "same").split(/\s+/)).toContain("h2");
      expect(headingClass(refRoot, "same").split(/\s+/)).toContain("h2");
    } finally {
      disposeRef();
      main.dispose();
    }
  });
});

describe("automatic heading serialization", () => {
  it("switches Markdown between ATX and auto forms without touching siblings", () => {
    loadSingle(page([
      plain("untouched"),
      { id: "target", raw: "### Title\nbody bytes", collapsed: false, children: [], heading_level: 3 },
    ]));

    setHeading("target", true);
    expect(doc.byId.target.raw).toBe("Title\nheading:: true\nbody bytes");
    expect(pageToDto("Headings")!.blocks[1].raw).toBe("Title\nheading:: true\nbody bytes");
    expect(doc.byId.untouched.raw).toBe("untouched");

    setHeading("target", 4);
    expect(doc.byId.target.raw).toBe("#### Title\nbody bytes");
    expect(pageToDto("Headings")!.blocks[1].raw).toBe("#### Title\nbody bytes");
    expect(pageToDto("Headings")!.blocks[0].raw).toBe("untouched");
  });

  it("switches Org between numeric and auto drawer properties without touching siblings", () => {
    loadSingle(page([
      plain("untouched-org"),
      {
        id: "target-org",
        raw: "Title\n:PROPERTIES:\n:heading: 3\n:END:\nbody bytes",
        collapsed: false,
        children: [],
        properties: [["heading", "3"]],
      },
    ], "org"));

    setHeading("target-org", true);
    expect(doc.byId["target-org"].raw).toBe("Title\n:PROPERTIES:\n:heading: true\n:END:\nbody bytes");
    expect(pageToDto("Headings")!.blocks[1].raw).toBe("Title\n:PROPERTIES:\n:heading: true\n:END:\nbody bytes");
    expect(doc.byId["untouched-org"].raw).toBe("untouched-org");

    setHeading("target-org", 4);
    expect(doc.byId["target-org"].raw).toBe("Title\n:PROPERTIES:\n:heading: 4\n:END:\nbody bytes");
    expect(pageToDto("Headings")!.blocks[1].raw).toBe("Title\n:PROPERTIES:\n:heading: 4\n:END:\nbody bytes");
    expect(pageToDto("Headings")!.blocks[0].raw).toBe("untouched-org");
  });
});
