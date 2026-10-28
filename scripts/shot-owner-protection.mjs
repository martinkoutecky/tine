// Native screenshots for the actual owner warning/recovery journey.
import { execFileSync } from "node:child_process";
export function shotOwnerProtection(file, env = process.env) {
  execFileSync("import", ["-window", "root", file], { env, timeout:10000 });
}
