// A fake StatusNotifier host for native Linux journeys (OG-TRAY, GH #625).
//
// Xvfb has no desktop shell, so no `org.kde.StatusNotifierWatcher` owns its
// name on the journey's private session bus and Tine correctly reports "no tray
// host". To drive the REAL tray path (the AppIndicator the app registers, and
// its menu over com.canonical.dbusmenu) this module owns that name itself, the
// way a panel does, with just enough of the protocol:
//
//   Properties.Get/GetAll   IsStatusNotifierHostRegistered, ProtocolVersion,
//                           RegisteredStatusNotifierItems
//   RegisterStatusNotifierItem / RegisterStatusNotifierHost   recorded / accepted
//
// It speaks the D-Bus wire protocol directly (EXTERNAL auth over the unix
// socket) because no D-Bus binding is installed in the harness. Menu clicks go
// the other way through the `gdbus` command line tool, so the journey presses
// the same menu entries a user would.
import net from "node:net";
import { execFileSync } from "node:child_process";

const WATCHER = "org.kde.StatusNotifierWatcher";

/** Little-endian D-Bus marshalling for the few shapes this host sends. */
class Writer {
  constructor() { this.bytes = []; }
  align(n) { while (this.bytes.length % n) this.bytes.push(0); }
  byte(v) { this.bytes.push(v); }
  u32(v) { this.align(4); for (let i = 0; i < 4; i++) this.bytes.push((v >>> (8 * i)) & 0xff); }
  str(s) { const b = Buffer.from(s, "utf8"); this.u32(b.length); this.bytes.push(...b, 0); }
  sig(s) { const b = Buffer.from(s, "ascii"); this.byte(b.length); this.bytes.push(...b, 0); }
  get length() { return this.bytes.length; }
  buffer() { return Buffer.from(this.bytes); }
}

/** Encode one message. `fields` is [[code, signature, writeValue]]. */
export function encodeMessage({ type, serial, fields, bodySignature = "", body = null }) {
  const bodyWriter = new Writer();
  if (body) body(bodyWriter);
  const allFields = bodySignature ? [...fields, [8, "g", (w) => w.sig(bodySignature)]] : fields;
  // The header field array starts at offset 16; its elements are 8-aligned.
  const header = new Writer();
  for (const [code, signature, write] of allFields) {
    header.align(8);
    header.byte(code);
    header.sig(signature);
    write(header);
  }
  const fieldBytes = header.buffer();
  const out = new Writer();
  out.byte(0x6c); // 'l'
  out.byte(type);
  out.byte(type === 1 ? 0x00 : 0x01); // replies and errors expect no reply
  out.byte(1);
  out.u32(bodyWriter.length);
  out.u32(serial);
  out.u32(fieldBytes.length);
  out.bytes.push(...fieldBytes);
  out.align(8);
  out.bytes.push(...bodyWriter.buffer());
  return out.buffer();
}

/** Parse the fixed header and the string-ish header fields of one message. */
export function decodeHeader(buf) {
  if (buf.length < 16) return null;
  if (buf[0] !== 0x6c) throw new Error("big-endian D-Bus peers are not supported");
  const bodyLength = buf.readUInt32LE(4);
  const fieldsLength = buf.readUInt32LE(12);
  const headerEnd = 16 + fieldsLength;
  const bodyStart = Math.ceil(headerEnd / 8) * 8;
  const total = bodyStart + bodyLength;
  if (buf.length < total) return null;
  const message = {
    type: buf[1], serial: buf.readUInt32LE(8), bodyStart, total,
    path: "", interface: "", member: "", sender: "", signature: "", replySerial: 0, errorName: "",
  };
  let off = 16;
  while (off < headerEnd) {
    off = Math.ceil(off / 8) * 8;
    if (off >= headerEnd) break;
    const code = buf[off];
    const sigLength = buf[off + 1];
    const sig = buf.toString("ascii", off + 2, off + 2 + sigLength);
    off += 3 + sigLength;
    if (sig === "u") {
      off = Math.ceil(off / 4) * 4;
      if (code === 5) message.replySerial = buf.readUInt32LE(off);
      off += 4;
    } else if (sig === "s" || sig === "o") {
      off = Math.ceil(off / 4) * 4;
      const length = buf.readUInt32LE(off);
      const text = buf.toString("utf8", off + 4, off + 4 + length);
      off += 5 + length;
      if (code === 1) message.path = text;
      else if (code === 2) message.interface = text;
      else if (code === 3) message.member = text;
      else if (code === 4) message.errorName = text;
      else if (code === 7) message.sender = text;
    } else if (sig === "g") {
      const length = buf[off];
      const text = buf.toString("ascii", off + 1, off + 1 + length);
      off += 2 + length;
      if (code === 8) message.signature = text;
    } else {
      throw new Error(`unexpected header field type ${sig}`);
    }
  }
  message.body = buf.subarray(bodyStart, total);
  return message;
}

const readString = (body, offset = 0) => {
  const at = Math.ceil(offset / 4) * 4;
  const length = body.readUInt32LE(at);
  return body.toString("utf8", at + 4, at + 4 + length);
};

/** The filesystem socket of the session bus. An abstract address is refused by
 * name: Node's net.connect cannot reach one and fails with a bare ECONNREFUSED
 * that reads as a dead bus. The runner's private bus listens on a path
 * (scripts/lib/e2e-session-bus.conf). */
export function busSocketPath(address = process.env.DBUS_SESSION_BUS_ADDRESS || "") {
  for (const candidate of address.split(";")) {
    const entry = candidate.replace(/^unix:/, "");
    const parts = Object.fromEntries(entry.split(",").map((kv) => kv.split("=")));
    if (parts.path) return parts.path;
  }
  throw new Error(`the session bus ${JSON.stringify(address)} has no filesystem socket: Node cannot connect to an abstract D-Bus socket; run the journey on the private bus (scripts/lib/e2e-session-bus.mjs)`);
}

/**
 * Own `org.kde.StatusNotifierWatcher` on the session bus. Resolves once the name
 * is ours. `items` fills as applications register status items.
 */
export async function startFakeTrayHost({ hostRegistered = true } = {}) {
  const socket = net.connect(busSocketPath());
  await new Promise((resolve, reject) => { socket.once("connect", resolve); socket.once("error", reject); });
  let pending = Buffer.alloc(0);
  let authorised = false;
  let serial = 1;
  const items = [];
  const waiting = new Map();
  const uid = Buffer.from(String(process.getuid())).toString("hex");
  const send = (bytes) => socket.write(bytes);

  const call = (destination, path, ifaceName, member, signature = "", body = null) => new Promise((resolve) => {
    const mine = serial++;
    waiting.set(mine, resolve);
    send(encodeMessage({
      type: 1, serial: mine,
      fields: [
        [1, "o", (w) => w.str(path)],
        [6, "s", (w) => w.str(destination)],
        [2, "s", (w) => w.str(ifaceName)],
        [3, "s", (w) => w.str(member)],
      ],
      bodySignature: signature, body,
    }));
  });

  const reply = (request, signature = "", body = null) => send(encodeMessage({
    type: 2, serial: serial++,
    fields: [[5, "u", (w) => w.u32(request.serial)], [6, "s", (w) => w.str(request.sender)]],
    bodySignature: signature, body,
  }));
  const fail = (request, name, text) => send(encodeMessage({
    type: 3, serial: serial++,
    fields: [[4, "s", (w) => w.str(name)], [5, "u", (w) => w.u32(request.serial)], [6, "s", (w) => w.str(request.sender)]],
    bodySignature: "s", body: (w) => w.str(text),
  }));

  const propertyVariant = (name) => {
    if (name === "IsStatusNotifierHostRegistered") return (w) => { w.sig("b"); w.u32(hostRegistered ? 1 : 0); };
    if (name === "ProtocolVersion") return (w) => { w.sig("i"); w.u32(0); };
    if (name === "RegisteredStatusNotifierItems") return (w) => {
      w.sig("as");
      const inner = new Writer();
      for (const item of items) inner.str(item.service);
      w.u32(inner.length);
      w.bytes.push(...inner.buffer());
    };
    return null;
  };

  const handleCall = (message) => {
    if (message.interface === "org.freedesktop.DBus.Peer") return reply(message);
    if (message.interface === "org.freedesktop.DBus.Properties" && message.member === "Get") {
      const name = readString(message.body, 4 + readString(message.body, 0).length + 1);
      const variant = propertyVariant(name);
      return variant ? reply(message, "v", variant) : fail(message, "org.freedesktop.DBus.Error.UnknownProperty", name);
    }
    if (message.interface === "org.freedesktop.DBus.Properties" && message.member === "GetAll") {
      return reply(message, "a{sv}", (w) => {
        const entries = new Writer();
        for (const name of ["IsStatusNotifierHostRegistered", "ProtocolVersion", "RegisteredStatusNotifierItems"]) {
          entries.align(8);
          entries.str(name);
          propertyVariant(name)(entries);
        }
        w.u32(entries.length);
        w.align(8);
        w.bytes.push(...entries.buffer());
      });
    }
    if (message.interface === WATCHER && message.member === "RegisterStatusNotifierItem") {
      items.push({ sender: message.sender, service: readString(message.body) });
      return reply(message);
    }
    if (message.interface === WATCHER && message.member === "RegisterStatusNotifierHost") return reply(message);
    return fail(message, "org.freedesktop.DBus.Error.UnknownMethod", `${message.interface}.${message.member}`);
  };

  socket.on("data", (chunk) => {
    pending = Buffer.concat([pending, chunk]);
    if (!authorised) {
      const end = pending.indexOf("\r\n");
      if (end < 0) return;
      if (!pending.toString("ascii", 0, end).startsWith("OK")) throw new Error(`D-Bus auth refused: ${pending.toString("ascii", 0, end)}`);
      pending = pending.subarray(end + 2);
      authorised = true;
      socket.write("BEGIN\r\n");
      authDone();
    }
    for (;;) {
      const message = decodeHeader(pending);
      if (!message) return;
      pending = pending.subarray(message.total);
      if (message.type === 1) handleCall(message);
      else if ((message.type === 2 || message.type === 3) && waiting.has(message.replySerial)) {
        waiting.get(message.replySerial)(message);
        waiting.delete(message.replySerial);
      }
    }
  });

  let authDone;
  const authenticated = new Promise((resolve) => { authDone = resolve; });
  socket.write(Buffer.concat([Buffer.from([0]), Buffer.from(`AUTH EXTERNAL ${uid}\r\n`)]));
  await authenticated;

  await call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "Hello");
  const granted = await call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName", "su",
    (w) => { w.str(WATCHER); w.u32(4); });
  // 1 = primary owner.
  if (granted.type !== 2 || granted.body.readUInt32LE(0) !== 1) throw new Error("could not own org.kde.StatusNotifierWatcher");

  return {
    items,
    close: () => socket.destroy(),
  };
}

const gdbus = (args, env = process.env) => execFileSync("gdbus", ["call", "--session", ...args], { env, encoding: "utf8", timeout: 10_000 });

/** The labels and ids of the menu a registered status item exports. */
export function readTrayMenu(item, env = process.env) {
  const menuPath = /'([^']+)'/.exec(gdbus([
    "--dest", item.sender, "--object-path", item.service,
    "--method", "org.freedesktop.DBus.Properties.Get", "org.kde.StatusNotifierItem", "Menu",
  ], env))?.[1];
  if (!menuPath) throw new Error(`status item ${item.service} exports no Menu`);
  const layout = gdbus(["--dest", item.sender, "--object-path", menuPath,
    "--method", "com.canonical.dbusmenu.GetLayout", "--", "0", "-1", "[]"], env);
  const entries = [];
  for (const match of layout.matchAll(/\((\d+), \{([^}]*)\}/g)) {
    const label = /'label': <'([^']*)'>/.exec(match[2])?.[1];
    if (label) entries.push({ id: Number(match[1]), label });
  }
  return { menuPath, entries };
}

/** Press the menu entry labelled `label`, as a click in the panel's menu does. */
export function clickTrayMenu(item, label, env = process.env) {
  const { menuPath, entries } = readTrayMenu(item, env);
  const entry = entries.find((candidate) => candidate.label.replace(/_/g, "") === label);
  if (!entry) throw new Error(`tray menu has no "${label}" entry: ${JSON.stringify(entries)}`);
  gdbus(["--dest", item.sender, "--object-path", menuPath,
    "--method", "com.canonical.dbusmenu.Event", String(entry.id), "clicked", "<int32 0>", "0"], env);
}
