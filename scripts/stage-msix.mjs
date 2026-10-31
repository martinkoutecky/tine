import fs from "node:fs";
import path from "node:path";
import { msixVersion } from "./lib/msix-version.mjs";
import { releaseChannel, STABLE_CHANNEL } from "./release-policy.mjs";
import { IDENTITIES, SHIP } from "./lib/app-identity.mjs";

// Data-preservation policy (Martin 2026-10-12, narrowed by D-20261031-MSV): Store and GitHub installs
// share Tine's three AppData folders through Windows 11 flexible virtualization. One edit removes the
// excluded directories and their restricted capability together.
const unvirtualized = true;
const conf = JSON.parse(fs.readFileSync("src-tauri/tauri.conf.json", "utf8"));
if (SHIP !== "release" || conf.identifier !== IDENTITIES.release.identifier) throw new Error("MSIX requires the release app identity");
if (releaseChannel(conf) !== STABLE_CHANNEL) throw new Error("MSIX requires the stable update channel");
const version = msixVersion(conf.version);
const template = fs.readFileSync("packaging/msix/AppxManifest.xml", "utf8");
// The excluded directories name the release data folders literally; they must follow the identity.
const excluded = [...template.matchAll(/<virtualization:ExcludedDirectory>([^<]+)<\/virtualization:ExcludedDirectory>/g)].map((m) => m[1]);
const expected = [`$(KnownFolder:RoamingAppData)\\${conf.identifier}`, `$(KnownFolder:LocalAppData)\\${conf.identifier}`, "$(KnownFolder:LocalAppData)\\tine-owners"];
if (JSON.stringify(excluded) !== JSON.stringify(expected)) throw new Error(`MSIX excluded directories ${JSON.stringify(excluded)} must be ${JSON.stringify(expected)}`);
for (const [arch, target] of [["x64", "x86_64-pc-windows-msvc"], ["arm64", "aarch64-pc-windows-msvc"]]) {
  const dir = path.join("msix-stage", arch);
  fs.mkdirSync(path.join(dir, "Assets"), { recursive: true });
  const buildDir = path.join("target", target, "release");
  const exe = [conf.productName + ".exe", "tine.exe"].map((name) => path.join(buildDir, name)).find(fs.existsSync);
  if (!exe) throw new Error(`main Tine executable missing in ${buildDir}`);
  fs.copyFileSync(exe, path.join(dir, "tine.exe"));
  for (const asset of ["StoreLogo.png", "Square44x44Logo.png", "Square150x150Logo.png"]) {
    fs.copyFileSync(path.join("src-tauri/icons", asset), path.join(dir, "Assets", asset));
  }
  let manifest = template.replaceAll("@VERSION@", version).replaceAll("@ARCH@", arch);
  if (!unvirtualized) manifest = manifest
    .replace(/\s*<!-- D-20261031-MSV[\s\S]*?<\/virtualization:FileSystemWriteVirtualization>/, "")
    .replace(/\s*<rescap:Capability Name="unvirtualizedResources" \/>/, "");
  fs.writeFileSync(path.join(dir, "AppxManifest.xml"), manifest);
}
console.log(`Staged ${version}: x64, arm64; unvirtualized=${unvirtualized}`);
