// Regenerate Beta assets on all platforms. Stable artwork is never overwritten.
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { chromium } from "playwright";
import { IDENTITIES } from "./lib/app-identity.mjs";

const artifacts = process.env.TINE_ICON_ARTIFACTS || "target/artifacts/beta-icon";
const iosOnly = process.argv.includes("--ios-only");
fs.mkdirSync(artifacts, { recursive: true });
const generated = fs.mkdtempSync(`${artifacts}/generated-`);
const run = (command, args) => execFileSync(command, args, { stdio: "inherit" });
run(process.execPath, ["scripts/render-icon.mjs", "docs/app-icon-beta.svg", `${artifacts}/beta-1024.png`]);
run("npx", ["tauri", "icon", `${artifacts}/beta-1024.png`, "-o", generated]);
fs.mkdirSync("src-tauri/icons-beta", { recursive: true });
for (const entry of fs.readdirSync(generated)) {
  if (iosOnly && entry !== "ios") continue;
  fs.cpSync(path.join(generated, entry), `src-tauri/icons-beta/${entry}`, { recursive: true });
}
// App Store catalogs reject PNG alpha even when every pixel is opaque. Tauri
// composites iOS artwork onto white; strip any redundant alpha channel.
run("python3", ["-c", `
from pathlib import Path
from PIL import Image
for icon in Path("src-tauri/icons-beta/ios").glob("*.png"):
    image = Image.open(icon).convert("RGB")
    image.save(icon)
`]);
if (!iosOnly) {
  run("npx", ["tauri", "icon", `${artifacts}/beta-1024.png`, "-o", "src-tauri/icons-beta", "-p", "64"]);
  run("python3", ["scripts/render-macos-icns.py", "docs/app-icon-beta-macos.svg", "src-tauri/icons-beta/icon.icns"]);
}

const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width: 640, height: 360 }, deviceScaleFactor: 1 });
  const data = (file) => `data:image/png;base64,${fs.readFileSync(file).toString("base64")}`;
  await page.setContent(`<body style="margin:0;background:#f1f5f9;font:20px sans-serif;display:flex;text-align:center;justify-content:space-evenly;padding-top:24px">
    <div><img width="256" height="256" src="${data("src-tauri/icons/ios/AppIcon-512@2x.png")}"><p>Tine · stable iOS</p></div>
    <div><img width="256" height="256" src="${data("src-tauri/icons-beta/ios/AppIcon-512@2x.png")}"><p>${IDENTITIES.experiment.productName} · iOS</p></div></body>`);
  await page.locator("img").evaluateAll((images) => Promise.all(images.map((image) => image.decode())));
  await page.screenshot({ path: `${artifacts}/stable-and-beta.png` });
} finally {
  await browser.close();
}
