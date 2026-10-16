import assert from "node:assert/strict";
import test from "node:test";
import { selectBetaRelease, servingProblems, zsyncAssetName } from "./pages-beta-manifest.mjs";

const repository = "martinkoutecky/tine";
const release = (tag_name, extra = {}) => ({ tag_name, draft: false, prerelease: true, ...extra });

test("selects the newest published Beta by version, never stable, drafts, the legacy tag or pre-beta.3 releases", () => {
  assert.equal(selectBetaRelease([release("v0.8.0-beta.1"), release("beta"), release("v0.7.3", { prerelease: false })]), null);
  const picked = selectBetaRelease([
    release("v0.8.0-beta.10"),
    release("v0.9.0-beta.1", { draft: true }),
    release("v0.8.0-beta.3"),
    release("v0.8.0-beta.9"),
    release("v0.8.0", { prerelease: true }),
    release("beta"),
  ]);
  assert.equal(picked.tag_name, "v0.8.0-beta.10");
});

const tag = "v0.8.0-beta.3";
const base = `https://github.com/${repository}/releases/download/${tag}/`;
const zsync = (arch, url = `${base}Tine-Beta_0.8.0-beta.3_${arch}.AppImage`) =>
  `zsync: 0.6.2\nFilename: Tine-Beta_0.8.0-beta.3_${arch}.AppImage\nURL: ${url}\nSHA-1: 00\n\n\u0001\u0002`;
const good = {
  repository,
  tag,
  manifest: { version: "0.8.0-beta.3", platforms: { "linux-x86_64": { url: `${base}Tine-Beta_0.8.0-beta.3_amd64.AppImage`, signature: "sig" } } },
  zsync: { amd64: zsync("amd64"), aarch64: zsync("aarch64") },
};

test("a release whose manifest and zsync files point into its own tag is servable", () => {
  assert.deepEqual(servingProblems(good), []);
  assert.equal(zsyncAssetName(tag, "amd64"), "Tine-Beta_0.8.0-beta.3_amd64.AppImage.zsync");
});

test("refuses manifests that escape the release, mismatch its version, lack signatures, or relative zsync URLs", () => {
  const legacy = { ...good, manifest: { ...good.manifest, platforms: { "linux-x86_64": { url: `https://github.com/${repository}/releases/download/beta/x.AppImage`, signature: "sig" } } } };
  assert.match(servingProblems(legacy).join("\n"), /does not point into v0\.8\.0-beta\.3/);
  assert.match(servingProblems({ ...good, manifest: { ...good.manifest, version: "0.8.0-beta.2" } }).join("\n"), /version is 0\.8\.0-beta\.2/);
  const unsigned = { ...good, manifest: { ...good.manifest, platforms: { "linux-x86_64": { url: `${base}a`, signature: "" } } } };
  assert.match(servingProblems(unsigned).join("\n"), /no signature/);
  assert.match(servingProblems({ ...good, zsync: { ...good.zsync, amd64: zsync("amd64", "Tine-Beta_0.8.0-beta.3_amd64.AppImage") } }).join("\n"), /amd64\.AppImage\.zsync URL is/);
  assert.match(servingProblems({ ...good, zsync: { amd64: good.zsync.amd64 } }).join("\n"), /aarch64\.AppImage\.zsync is missing/);
});
