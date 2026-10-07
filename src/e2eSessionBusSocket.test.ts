import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import { describe, expect, it } from "vitest";
import { PRIVATE_BUS_CONFIG } from "../scripts/lib/e2e-session-bus.mjs";
import { busSocketPath } from "../scripts/lib/e2e-tray-host.mjs";

// REG-OG-CATFIX-TRAY-BUS. Hosted Ubuntu's session.conf listens on `unix:tmpdir=`,
// which is an ABSTRACT socket there; Node's net.connect cannot reach one and fails
// with a bare ECONNREFUSED (hosted tray journey, 0.8 s in). The private bus every
// native journey runs on therefore listens on a filesystem socket, and a Node
// client that is handed an abstract address says so instead of "refused".
const rule = "the private E2E bus must listen on a filesystem socket (a Node client cannot connect to an abstract one); see scripts/lib/e2e-session-bus.conf";
const hasDbusDaemon = spawnSync("dbus-daemon", ["--version"], { stdio: "ignore" }).status === 0;

describe("private E2E session bus socket", () => {
  it("is configured to listen on a filesystem socket, never an abstract one", () => {
    const config = fs.readFileSync(PRIVATE_BUS_CONFIG, "utf8");
    const listens = [...config.matchAll(/<listen>([^<]*)<\/listen>/g)].map((match) => match[1]);
    expect(listens, rule).toEqual(["unix:dir=/tmp"]);
  });

  it("names an abstract address instead of reporting a refused connection", () => {
    expect(busSocketPath("unix:path=/tmp/dbus-x,guid=1")).toBe("/tmp/dbus-x");
    expect(busSocketPath("unix:abstract=/tmp/dbus-y,guid=1;unix:path=/tmp/dbus-z")).toBe("/tmp/dbus-z");
    expect(() => busSocketPath("unix:abstract=/tmp/dbus-y,guid=1"), rule).toThrow("abstract D-Bus socket");
    expect(() => busSocketPath(""), rule).toThrow("no filesystem socket");
  });

  it.skipIf(!hasDbusDaemon)("a real bus started with the private config is reachable from Node", async () => {
    const daemon = spawn("dbus-daemon", [`--config-file=${PRIVATE_BUS_CONFIG}`, "--nofork", "--print-address"], {
      stdio: ["ignore", "pipe", "ignore"],
    });
    try {
      const address = await new Promise<string>((resolve, reject) => {
        let seen = "";
        daemon.stdout!.on("data", (chunk) => {
          seen += chunk;
          if (seen.includes("\n")) resolve(seen.trim());
        });
        daemon.once("error", reject);
        daemon.once("exit", () => reject(new Error(`dbus-daemon exited before printing an address: ${seen}`)));
      });
      const socketPath = busSocketPath(address.split(",guid")[0] + ",guid=x");
      await new Promise<void>((resolve, reject) => {
        const socket = net.connect(socketPath);
        socket.once("connect", () => { socket.destroy(); resolve(); });
        socket.once("error", reject);
      });
    } finally {
      daemon.kill("SIGKILL");
    }
  });
});
