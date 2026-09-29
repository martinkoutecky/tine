import { describe, expect, it } from "vitest";
import { PLUGIN_CAPABILITIES } from "./manifest";

// This is a DELIBERATE change-detector, and the only one in the plugin suite.
//
// Apple App Review guideline 4.7.2 forbids plug-ins that "extend or expose native
// platform APIs or technologies to the software without prior permission from
// Apple". Tine's plugin API is compliant by construction — every capability is a
// semantic verb the host performs on the guest's behalf — but nothing except this
// test makes a future author notice before widening it.
//
// So: adding a capability is supposed to fail here. That is the point. When it
// does, read docs/adr/0052-ios-plugin-platform-apple-4-7-2.md, confirm the new
// capability is host-mediated and exposes no native surface, then add it below.
//
// Do NOT "fix" this by deleting the test or loosening it to a length check.
const HOST_MEDIATED_CAPABILITIES = [
  "commands.register",
  "slash-commands.register",
  "block-decorations.register",
  "graph.read.visible",
  "graph.write.block",
  "settings.read",
  "settings.write",
] as const;

describe("plugin capability boundary (ADR 0052 / Apple 4.7.2)", () => {
  it("exposes no native platform surface to guests", () => {
    expect(
      [...PLUGIN_CAPABILITIES].sort(),
      "PLUGIN_CAPABILITIES changed. Read docs/adr/0052-ios-plugin-platform-apple-4-7-2.md. " +
        "A capability must be a semantic verb the HOST performs for the guest — never Tauri " +
        "`invoke`, raw filesystem/path handles, process, sockets, shell, OS services, or any " +
        "pass-through that lets a guest name a native command. One such capability forfeits " +
        "plugins on iOS permanently, and a desktop-only escape hatch does not help: it would " +
        "have to be removed (stranding plugins built on it) before an iOS host could ship. " +
        "If the new capability is host-mediated, add it to HOST_MEDIATED_CAPABILITIES here.",
    ).toEqual([...HOST_MEDIATED_CAPABILITIES].sort());
  });
});
