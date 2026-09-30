import { beforeAll } from "vitest";

// Structural helpers are synchronous after the same initialization awaited by
// main.tsx/capture.tsx. Exercise the parser door in every harness too.
beforeAll(async () => {
  // Import after test modules register their wasm/init-failure mocks.
  const { initParser } = await import("./render/parse");
  await initParser();
});
