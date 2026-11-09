import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";
import renderConfig from "./vitest.render.config";
import { testFilesObservingSetup } from "./scripts/lib/test-import-graph.mjs";

// Opt-in census: the render suite again, with its normal setup plus the CM6 setup,
// which forces the CM6 editor on. Only a file whose import graph reaches what that
// setup overrides can see the difference, so only those run here; every other file
// would repeat its render-pass result. The list is derived from the imports on each
// run, so a new test that mounts an editor joins it by importing one.
const setupFile = "./src/tests/setup.cm6.ts";
const test = renderConfig.test!;

export default defineConfig({
  ...renderConfig,
  test: {
    ...test,
    setupFiles: [...(test.setupFiles as string[]), setupFile],
    include: testFilesObservingSetup({ root: fileURLToPath(new URL(".", import.meta.url)), include: test.include!, setupFile }),
  },
});
