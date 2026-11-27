import { expect, it } from "vitest";
import { FileConsentAsk, fileConsentAsk } from "./fileConsentWire";

// tine#985: only the exact native token is an Ask (the shape is pinned on the
// Rust side by outside_file_tests the_ask_wire_is_the_exact_token...).
it("decodes the exact native Ask token and nothing else", () => {
  const ask = fileConsentAsk(new Error(String.raw`file-consent:{"display":"/T/a \"b\"\\c\u00e9�","lossy":true}`));
  expect(ask).toBeInstanceOf(FileConsentAsk);
  expect(ask).toMatchObject({ display: "/T/a \"b\"\\cé�", lossy: true });
  expect(fileConsentAsk(`file-consent:${JSON.stringify({ display: "/T/x", lossy: false })}`)).toMatchObject({ display: "/T/x", lossy: false });
  for (const other of [
    "outside-file", "file-consent:", "file-consent:{}", "file-consent:{\"display\":\"/T/x\"}",
    String.raw`file-consent:{"display":"\q","lossy":false}`, "file-consent:{\"display\":\"/T/x\",\"lossy\":false} ",
    "x file-consent:{\"display\":\"/T/x\",\"lossy\":false}", new Error("io:NotFound"),
  ]) expect(fileConsentAsk(other)).toBeNull();
});
