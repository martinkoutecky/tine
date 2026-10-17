import assert from "node:assert/strict";
import test from "node:test";
import { IDENTITIES } from "./lib/app-identity.mjs";
import { releaseLayout } from "./release-layout.mjs";
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
const platforms = () => Object.fromEntries(Object.entries(releaseLayout("0.8.0-beta.3", IDENTITIES.experiment).updaterPlatforms)
  .map(([platform, [asset]]) => [platform, { url: `${base}${asset}`, signature: "sig" }]));
const good = () => ({
  repository,
  tag,
  manifest: { version: "0.8.0-beta.3", platforms: platforms() },
  zsync: { amd64: zsync("amd64"), aarch64: zsync("aarch64") },
});
const withPlatforms = (edit) => {
  const value = good();
  edit(value.manifest.platforms);
  return value;
};

test("a complete Beta manifest whose assets and zsync files are in its own tag is servable", () => {
  assert.deepEqual(servingProblems(good()), []);
  assert.equal(zsyncAssetName(tag, "amd64"), "Tine-Beta_0.8.0-beta.3_amd64.AppImage.zsync");
  // A Beta may add an updater target before this branch's layout knows it.
  assert.deepEqual(servingProblems(withPlatforms((p) => { p["darwin-new"] = { url: `${base}Tine-Beta_0.8.0-beta.3_new.tar.gz`, signature: "s" }; })), []);
});

test("refuses incomplete, crossed, wrong-identity, wrong-version, unsigned or escaping manifests", () => {
  const refuse = (value, pattern) => assert.match(servingProblems(value).join("\n"), pattern);
  refuse(withPlatforms((p) => { for (const key of Object.keys(p)) if (key !== "linux-x86_64") delete p[key]; }), /is missing linux-aarch64/);
  refuse(withPlatforms((p) => { p["linux-aarch64"].url = p["linux-x86_64"].url; }), /linux-aarch64 is .*amd64/);
  refuse(withPlatforms((p) => { for (const entry of Object.values(p)) entry.url = entry.url.replace("Tine-Beta_0.8.0-beta.3", "Tine_0.7.4").replace("Tine-Beta-0.8.0-beta.3", "Tine-0.7.4"); }), /expected/);
  refuse(withPlatforms((p) => { p["linux-x86_64"].url = base; }), /linux-x86_64 is/);
  refuse(withPlatforms((p) => { p["darwin-new"] = { url: `${base}Tine_0.8.0-beta.3_new.tar.gz`, signature: "s" }; }), /darwin-new is/);
  refuse(withPlatforms((p) => { p["linux-x86_64"].url = p["linux-x86_64"].url.replace(tag, "beta"); }), /linux-x86_64 is/);
  refuse(withPlatforms((p) => { p["linux-x86_64"].signature = ""; }), /no signature/);
  refuse({ ...good(), manifest: { ...good().manifest, version: "0.8.0-beta.2" } }, /version is 0\.8\.0-beta\.2/);
  refuse({ ...good(), zsync: { ...good().zsync, amd64: zsync("amd64", "Tine-Beta_0.8.0-beta.3_amd64.AppImage") } }, /amd64\.AppImage\.zsync URL is/);
  refuse({ ...good(), zsync: { amd64: good().zsync.amd64 } }, /aarch64\.AppImage\.zsync is missing/);
});
