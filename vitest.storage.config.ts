import { configDefaults, defineConfig } from "vitest/config";
import base from "./vitest.config";

// Storage conformance (F1/F6): the real TS HostClient against a real native
// PageHost peer, which this suite builds with cargo. Kept out of the default
// frontend run; CI's Linux jobs and the batch gate run it after the Rust tests.
// (mergeConfig would concatenate `include`, so the base is spread instead.)
export default defineConfig({
  ...base,
  test: {
    ...base.test,
    include: ["src/document/host/nativeConformance.test.ts"],
    exclude: [...configDefaults.exclude],
  },
});
