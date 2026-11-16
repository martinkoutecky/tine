import fs from "node:fs";
import path from "node:path";
import { checkViewer } from "./check-quick-share-viewer.mjs";

/** Returns an attestation only after the deployed, hash-verified files have
 * rendered the candidate's entire fixture corpus. No network writes. */
export async function checkQuickShareReadiness() {
  const out = path.resolve(import.meta.dirname, "../artifacts/quick-share-viewer");
  fs.mkdirSync(out, { recursive: true });
  const envFile = path.join(out, "writer-ready.env");
  fs.rmSync(envFile, { force: true }); // A failed recheck must not leave an old attestation.
  const fingerprint = await checkViewer("https://tine.page/s/1/");
  if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT, `quick_share_viewer_g1_ready=${fingerprint}\n`);
  fs.writeFileSync(envFile, `TINE_QUICK_SHARE_VIEWER_G1_READY=${fingerprint}\n`);
  return fingerprint;
}
