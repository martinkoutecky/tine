// A synthetic updater signature for release-script tests: the base64 minisign
// signature file the Tauri bundler writes, with fake signature lines. Its
// trusted comment names `version` when given, as the bundler's does.
export function fixtureSignature(file, version) {
  const comment = [`timestamp:1700000000`, `file:${file}`, ...(version === undefined ? [] : [`version:${version}`])].join("\t");
  const text = `untrusted comment: signature from tauri secret key\nRUQfixture\ntrusted comment: ${comment}\nfixture\n`;
  return Buffer.from(text).toString("base64");
}
