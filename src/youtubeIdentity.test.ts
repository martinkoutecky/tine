import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { expectOrderedIn } from "./testOrder.test.support";

const read = (file: string) => readFileSync(new URL(`../src-tauri/src/${file}`, import.meta.url), "utf8");

describe("native YouTube identity setup (I-21)", () => {
  it("names all five shipped platforms without treating an unproven target as unsupported", () => {
    const setup = read("youtube_identity.rs");
    for (const target of ["linux", "windows", "macos", "android", "ios"]) {
      expect(setup).toContain(`target_os = "${target}"`);
    }
    expect(setup).not.toContain("cfg(not(");
  });

  it("configures initial and later graph WebViews before their first navigation", () => {
    const main = read("lib.rs");
    const graph = read("graph.rs");
    expect(main).toContain("youtube_identity::prepare(&mut context)");
    expect(main).toContain("youtube_identity::create_windows(app, &youtube_windows);");
    expect(graph).toContain("youtube_identity::configure(builder, app)");
    expectOrderedIn(main, "youtube_identity::create_windows(app, &youtube_windows);", "graph::prepare_startup_graph(app.handle())");
    expectOrderedIn(graph, "youtube_identity::configure(builder, app)", "let built = builder.build()");
    expect(main).toContain("youtube_identity::cleanup(app)");
  });
});
