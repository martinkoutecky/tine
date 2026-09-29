import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "../../render/parse";
import { parseLsdocDocument } from "./lsdoc-document";
import { isIntentionalNestedDollarDivergence } from "./orchestrator";

beforeAll(async () => {
  await initParser();
});

describe("intentional lsdoc divergence classification", () => {
  it("suppresses only the complete nested-dollar-in-emphasis delta", async () => {
    const input = "- **999$a$a**";
    const original = parseLsdocDocument(input, false);
    const parsedTexts: string[] = [];

    const intentional = await isIntentionalNestedDollarDivergence(
      input,
      "md",
      original,
      async (text) => {
        parsedTexts.push(text);
        const projection = parseLsdocDocument(text, false);
        return {
          ok: true,
          diverges: false,
          lsdocProjection: projection,
          mldocProjection: projection,
        };
      },
    );

    expect(intentional).toBe(true);
    expect(parsedTexts).toEqual(["$a$", "- **999,,,a**"]);
  });

  it("fails closed when the isolated dollar span still diverges", async () => {
    const input = "- **999$a$a**";
    const original = parseLsdocDocument(input, false);
    const intentional = await isIntentionalNestedDollarDivergence(
      input,
      "md",
      original,
      async (text) => ({
        ok: true,
        diverges: text === "$a$",
        lsdocProjection: parseLsdocDocument(text, false),
        mldocProjection: parseLsdocDocument(text, false),
      }),
    );

    expect(intentional).toBe(false);
  });
});
