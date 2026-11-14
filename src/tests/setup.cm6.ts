import { vi } from "vitest";
import "../testSetup.range";

// Run the editor-neutral suite with CM6; the ordinary render setup selects classic.
vi.mock("../editor/cm6Setting", async importOriginal => ({
  ...await importOriginal<typeof import("../editor/cm6Setting")>(),
  cm6EditorEnabled: () => true,
}));

// Dual-engine files (useEditorKind) already run their CM6 half in the ordinary
// render suite; their textarea half cannot run here, where CM6 is forced on.
(globalThis as { __tineCm6Suite?: boolean }).__tineCm6Suite = true;
