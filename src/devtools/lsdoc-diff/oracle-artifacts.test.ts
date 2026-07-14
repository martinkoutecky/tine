import { describe, expect, it } from "vitest";
import {
  isMldocBacktickStateArtifact,
  mldocBacktickArtifactSourceSpan,
  shouldQuarantineMldocBacktickStateArtifact,
} from "./oracle-artifacts";

const refs = { page: [] as string[], block: [] as string[] };

describe("mldoc oracle artifact classification", () => {
  it("recognizes only the issue #82 plain/code backtick ownership shift", () => {
    const lsdoc = {
      blocks: [{
        kind: "paragraph",
        span: [0, 20],
        inline: [
          { k: "plain", text: "ä ", span: [0, 3] },
          { k: "code", text: "`aaaa\nä {", span: [3, 20] },
          { k: "plain", text: "`" },
        ],
      }],
      refs,
    };
    const mldoc = {
      blocks: [{
        kind: "paragraph",
        inline: [
          { k: "plain", text: "ä `" },
          { k: "code", text: "aaaa\nä {" },
          { k: "plain", text: "`" },
        ],
      }],
      refs,
    };
    expect(isMldocBacktickStateArtifact(lsdoc, mldoc)).toBe(true);
    expect(mldocBacktickArtifactSourceSpan(lsdoc, mldoc)).toEqual([0, 20]);
    expect(shouldQuarantineMldocBacktickStateArtifact(false, lsdoc, mldoc)).toBe(false);
    expect(shouldQuarantineMldocBacktickStateArtifact(true, lsdoc, mldoc)).toBe(true);
  });

  it("compares the artifact in canonical JSON space where undefined object fields are omitted", () => {
    const lsdoc = {
      blocks: [{
        kind: "list",
        items: [{
          checkbox: undefined,
          content: [
            { k: "plain", text: "before " },
            { k: "code", text: "`payload" },
          ],
        }],
      }],
      refs,
    };
    const mldoc = {
      blocks: [{
        kind: "list",
        items: [{
          content: [
            { k: "plain", text: "before `" },
            { k: "code", text: "payload" },
          ],
        }],
      }],
      refs,
    };

    const jsonItem = JSON.parse(JSON.stringify(lsdoc)).blocks[0].items[0];
    expect(Object.hasOwn(jsonItem, "checkbox")).toBe(false);
    expect(isMldocBacktickStateArtifact(lsdoc, mldoc)).toBe(true);
  });

  it("does not hide ordinary code-content or structural divergences", () => {
    const base = { blocks: [{ kind: "paragraph", inline: [{ k: "code", text: "a" }] }], refs };
    const differentCode = { blocks: [{ kind: "paragraph", inline: [{ k: "code", text: "b" }] }], refs };
    const differentKind = { blocks: [{ kind: "heading", inline: [{ k: "code", text: "a" }] }], refs };
    expect(isMldocBacktickStateArtifact(base, differentCode)).toBe(false);
    expect(isMldocBacktickStateArtifact(base, differentKind)).toBe(false);
    expect(isMldocBacktickStateArtifact(base, base)).toBe(false);
    expect(mldocBacktickArtifactSourceSpan(base, differentCode)).toBeNull();
  });
});
