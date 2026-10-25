import { mergeConfig } from "vitest/config";
import renderConfig from "./vitest.render.config";

// Branch-only census: inherit the complete render suite and its normal setup.
export default mergeConfig(renderConfig, {
  test: { setupFiles: ["./src/tests/setup.cm6.ts"] },
});
