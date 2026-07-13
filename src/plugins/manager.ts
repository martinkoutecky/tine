import { createSignal } from "solid-js";
import { backend, type InstalledPluginRecord } from "../backend";
import { doc, setRaw } from "../store";
import { pushToast } from "../ui";
import { platformKind } from "../platform";
import {
  parsePluginManifest,
  supportsPlatform,
  type PluginCommandContribution,
  type PluginManifest,
  type PluginPlatform,
  type PluginSlashCommandContribution,
} from "./manifest";
import {
  PLUGIN_PROTOCOL_VERSION,
  type PluginBlockSnapshot,
  type PluginEffect,
  type PluginEvent,
} from "./protocol";
import { PluginRuntime } from "./runtime";
import {
  defaultPluginSettings,
  parsePluginSettingsBlob,
  settingAccepts,
  validatePluginSettings,
  type PluginSettingValue,
  type PluginSettings,
} from "./settings";

export interface ManagedPlugin {
  manifest: PluginManifest;
  storageId: string;
  storageVersion: string;
  sha256: string;
  selected: boolean;
  enabled: boolean;
  running: boolean;
  settings: PluginSettings;
  error?: string;
}

export interface ManagedCommand {
  pluginId: string;
  contribution: PluginCommandContribution;
}

export interface ManagedSlashCommand {
  pluginId: string;
  contribution: PluginSlashCommandContribution;
}

type ActivePlugin = {
  manifest: PluginManifest;
  runtime: PluginRuntime;
};

export type RevokedPluginVersions = ReadonlySet<string>;

const [installedPlugins, setInstalledPlugins] = createSignal<ManagedPlugin[]>([]);
export { installedPlugins };

function versionKey(id: string, version: string): string {
  return `${id}@${version}`;
}

function displayError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function digestHex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytesBuffer(bytes));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function bytesBuffer(bytes: Uint8Array): ArrayBuffer {
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
}

export class PluginManager {
  private readonly active = new Map<string, ActivePlugin>();
  private platform: PluginPlatform = "desktop";
  private revoked: RevokedPluginVersions = new Set();

  async initialize(revoked: RevokedPluginVersions = new Set()) {
    this.platform = await platformKind();
    this.revoked = revoked;
    const records = await backend().listInstalledPlugins();
    const managed = await Promise.all(records.map(async (record) => {
      const plugin = this.parseRecord(record);
      if (!plugin.error) plugin.settings = await this.loadSettings(plugin.manifest);
      return plugin;
    }));
    setInstalledPlugins(managed);
    for (const plugin of managed) {
      if (plugin.enabled) await this.start(plugin.manifest.id, plugin.manifest.version, false);
    }
  }

  async install(manifestValue: unknown, wasm: Uint8Array): Promise<ManagedPlugin> {
    const manifest = parsePluginManifest(manifestValue);
    if (!supportsPlatform(manifest, this.platform)) {
      throw new Error(`${manifest.name} does not support ${this.platform}`);
    }
    if (this.revoked.has(versionKey(manifest.id, manifest.version))) {
      throw new Error("this plugin version has been revoked");
    }
    // Compile and instantiate inside the worker before persisting. A start function
    // is still bounded by the initialization timeout and cannot gain host imports.
    const proof = await PluginRuntime.create(bytesBuffer(wasm));
    proof.dispose();
    const record = await backend().installPlugin(JSON.stringify(manifest), wasm);
    const plugin = this.parseRecord(record);
    plugin.settings = await this.loadSettings(manifest);
    setInstalledPlugins((current) => [
      ...current.filter((item) => versionKey(item.manifest.id, item.manifest.version) !== versionKey(manifest.id, manifest.version)),
      plugin,
    ]);
    return plugin;
  }

  async enable(id: string, version: string): Promise<void> {
    await this.start(id, version, true);
  }

  async disable(id: string): Promise<void> {
    this.active.get(id)?.runtime.dispose();
    this.active.delete(id);
    const current = installedPlugins().find((plugin) => plugin.manifest.id === id && plugin.selected);
    if (current) await backend().setPluginEnabled(id, current.manifest.version, false);
    this.patch(id, undefined, { enabled: false, running: false, error: undefined });
  }

  async uninstall(id: string, version: string): Promise<void> {
    const target = installedPlugins().find(
      (plugin) => plugin.manifest.id === id && plugin.manifest.version === version
    );
    if (!target) throw new Error("plugin version is not installed");
    const active = this.active.get(id);
    if (active?.manifest.version === version) {
      active.runtime.dispose();
      this.active.delete(id);
      this.patch(id, version, { enabled: false, running: false, error: undefined });
    }
    await backend().uninstallPlugin(target.storageId, target.storageVersion);
    const remaining = installedPlugins().filter(
      (item) => versionKey(item.storageId, item.storageVersion) !== versionKey(target.storageId, target.storageVersion)
    );
    setInstalledPlugins(remaining);
    if (!remaining.some((item) => item.storageId === target.storageId)) {
      await backend().setAppString(this.settingsStorageKey(target.storageId), "{}");
    }
  }

  async setSetting(id: string, version: string, key: string, value: PluginSettingValue): Promise<void> {
    const plugin = installedPlugins().find(
      (item) => item.manifest.id === id && item.manifest.version === version
    );
    if (!plugin) throw new Error("plugin version is not installed");
    const definition = plugin.manifest.settings?.find((item) => item.key === key);
    if (!definition || !settingAccepts(definition, value)) throw new Error("plugin setting value is invalid");
    const settings = { ...plugin.settings, [key]: value };
    await this.storeSettings(plugin.manifest, settings, [key], true);
  }

  async resetSetting(id: string, version: string, key: string): Promise<void> {
    const plugin = installedPlugins().find(
      (item) => item.manifest.id === id && item.manifest.version === version
    );
    if (!plugin) throw new Error("plugin version is not installed");
    const definition = plugin.manifest.settings?.find((item) => item.key === key);
    if (!definition) throw new Error("plugin setting does not exist");
    await this.storeSettings(plugin.manifest, { ...plugin.settings, [key]: definition.default }, [key], true);
  }

  async resetSettings(id: string, version: string): Promise<void> {
    const plugin = installedPlugins().find(
      (item) => item.manifest.id === id && item.manifest.version === version
    );
    if (!plugin) throw new Error("plugin version is not installed");
    const settings = defaultPluginSettings(plugin.manifest.settings);
    await this.storeSettings(plugin.manifest, settings, (plugin.manifest.settings ?? []).map((item) => item.key), true);
  }

  commands(): ManagedCommand[] {
    // Track installation/enablement reactively for shortcut registration and
    // command-palette consumers.
    installedPlugins();
    const commands: ManagedCommand[] = [];
    for (const { manifest } of this.active.values()) {
      for (const contribution of manifest.contributions?.commands ?? []) {
        if (supportsPlatform(manifest, this.platform, contribution.platforms)) {
          commands.push({ pluginId: manifest.id, contribution });
        }
      }
    }
    return commands;
  }

  slashCommands(): ManagedSlashCommand[] {
    const commands: ManagedSlashCommand[] = [];
    for (const { manifest } of this.active.values()) {
      for (const contribution of manifest.contributions?.slashCommands ?? []) {
        if (supportsPlatform(manifest, this.platform, contribution.platforms)) {
          commands.push({ pluginId: manifest.id, contribution });
        }
      }
    }
    return commands;
  }

  hasDeclarativeDecoration(kind: "thread-lines" | "badge"): boolean {
    // Read the signal so block views update when a plugin is enabled or disabled.
    installedPlugins();
    for (const { manifest } of this.active.values()) {
      if (
        manifest.contributions?.blockDecorations?.some(
          (contribution) => contribution.kind === kind && supportsPlatform(manifest, this.platform, contribution.platforms)
        )
      ) {
        return true;
      }
    }
    return false;
  }

  declarativeDecorationSetting(
    kind: "thread-lines" | "badge",
    key: string
  ): PluginSettingValue | undefined {
    // Settings are held in the same signal as enablement so decoration hosts
    // rerender immediately without consulting or trusting guest output.
    const managed = installedPlugins();
    for (const { manifest } of this.active.values()) {
      if (!manifest.contributions?.blockDecorations?.some(
        (contribution) => contribution.kind === kind && supportsPlatform(manifest, this.platform, contribution.platforms)
      )) continue;
      return managed.find((plugin) =>
        plugin.manifest.id === manifest.id && plugin.manifest.version === manifest.version
      )?.settings[key];
    }
    return undefined;
  }

  async invokeCommand(pluginId: string, contributionId: string, focusedBlock?: PluginBlockSnapshot) {
    const plugin = this.active.get(pluginId);
    if (!plugin) throw new Error("plugin is not running");
    const contribution = plugin.manifest.contributions?.commands?.find((item) => item.id === contributionId);
    if (!contribution || !supportsPlatform(plugin.manifest, this.platform, contribution.platforms)) {
      throw new Error("plugin command is unavailable");
    }
    const event: PluginEvent = {
      protocolVersion: PLUGIN_PROTOCOL_VERSION,
      kind: "command",
      contributionId,
      ...(focusedBlock ? { focusedBlock } : {}),
    };
    await this.invokeAndApply(plugin, event);
  }

  async invokeSlashCommand(
    pluginId: string,
    contributionId: string,
    focusedBlock: PluginBlockSnapshot
  ): Promise<PluginEffect[]> {
    const plugin = this.active.get(pluginId);
    if (!plugin) throw new Error("plugin is not running");
    const contribution = plugin.manifest.contributions?.slashCommands?.find((item) => item.id === contributionId);
    if (!contribution || !supportsPlatform(plugin.manifest, this.platform, contribution.platforms)) {
      throw new Error("plugin slash command is unavailable");
    }
    return this.invokeAndApply(plugin, {
      protocolVersion: PLUGIN_PROTOCOL_VERSION,
      kind: "slash-command",
      contributionId,
      focusedBlock,
    });
  }

  async decorateBlocks(pluginId: string, contributionId: string, blocks: PluginBlockSnapshot[]): Promise<PluginEffect[]> {
    const plugin = this.active.get(pluginId);
    if (!plugin) return [];
    const contribution = plugin.manifest.contributions?.blockDecorations?.find((item) => item.id === contributionId);
    if (!contribution || !supportsPlatform(plugin.manifest, this.platform, contribution.platforms)) return [];
    const event: PluginEvent = {
      protocolVersion: PLUGIN_PROTOCOL_VERSION,
      kind: "decorate-blocks",
      contributionId,
      blocks,
    };
    return this.invokeAndApply(plugin, event);
  }

  async applyRevocations(revoked: RevokedPluginVersions) {
    this.revoked = revoked;
    for (const [id, active] of this.active) {
      if (revoked.has(versionKey(id, active.manifest.version))) {
        await this.disable(id);
        this.patch(id, active.manifest.version, { error: "This version was revoked by the registry." });
      }
    }
  }

  private parseRecord(record: InstalledPluginRecord): ManagedPlugin {
    try {
      const manifest = parsePluginManifest(JSON.parse(record.manifest_json));
      const incompatible = !supportsPlatform(manifest, this.platform);
      const revoked = this.revoked.has(versionKey(manifest.id, manifest.version));
      return {
        manifest,
        storageId: record.id,
        storageVersion: record.version,
        sha256: record.sha256,
        selected: record.selected,
        enabled: record.enabled && !incompatible && !revoked,
        running: false,
        settings: defaultPluginSettings(manifest.settings),
        ...(incompatible ? { error: `Not available on ${this.platform}.` } : {}),
        ...(revoked ? { error: "This version was revoked by the registry." } : {}),
      };
    } catch (error) {
      return {
        manifest: {
          schemaVersion: 1,
          id: `invalid.${record.sha256.slice(0, 12) || "plugin"}`,
          name: "Invalid plugin",
          version: "0.0.0",
          apiVersion: "0.2",
          description: "The installed manifest could not be validated.",
          author: "Unknown",
          license: "Unknown",
          source: "about:blank",
          entry: "plugin.wasm",
          platforms: ["desktop"],
          capabilities: [],
        },
        storageId: record.id,
        storageVersion: record.version,
        sha256: record.sha256,
        selected: false,
        enabled: false,
        running: false,
        settings: {},
        error: displayError(error),
      };
    }
  }

  private async start(id: string, version: string, persist: boolean) {
    const plugin = installedPlugins().find(
      (item) => item.manifest.id === id && item.manifest.version === version
    );
    if (!plugin) throw new Error("plugin version is not installed");
    if (!supportsPlatform(plugin.manifest, this.platform)) throw new Error(`plugin does not support ${this.platform}`);
    if (this.revoked.has(versionKey(id, version))) throw new Error("this plugin version has been revoked");
    try {
      const bytes = await backend().readPluginEntry(id, version);
      if (plugin.sha256 !== "mock" && (await digestHex(bytes)) !== plugin.sha256) {
        throw new Error("installed plugin digest does not match its recorded bytes");
      }
      const runtime = await PluginRuntime.create(bytesBuffer(bytes));
      const settings = await this.loadSettings(plugin.manifest);
      this.patchSettings(id, settings);
      const active: ActivePlugin = { manifest: plugin.manifest, runtime };
      await this.invokeAndApply(active, {
        protocolVersion: PLUGIN_PROTOCOL_VERSION,
        kind: "activate",
        platform: this.platform,
        capabilities: plugin.manifest.capabilities,
        settings: plugin.manifest.capabilities.includes("settings.read") ? settings : {},
      });
      this.active.get(id)?.runtime.dispose();
      this.active.set(id, active);
      if (persist) await backend().setPluginEnabled(id, version, true);
      setInstalledPlugins((current) =>
        current.map((item) =>
          item.manifest.id !== id
            ? item
            : {
                ...item,
                selected: item.manifest.version === version,
                enabled: item.manifest.version === version,
                running: item.manifest.version === version,
                error: undefined,
              }
        )
      );
    } catch (error) {
      if (persist || plugin.enabled) await backend().setPluginEnabled(id, version, false);
      this.patch(id, version, { enabled: false, running: false, error: displayError(error) });
      throw error;
    }
  }

  private async invokeAndApply(plugin: ActivePlugin, event: PluginEvent): Promise<PluginEffect[]> {
    let effects: PluginEffect[];
    try {
      effects = (await plugin.runtime.invoke(event)).effects;
    } catch (error) {
      await this.disable(plugin.manifest.id);
      this.patch(plugin.manifest.id, plugin.manifest.version, { error: displayError(error) });
      throw error;
    }
    const accepted: PluginEffect[] = [];
    for (const effect of effects) {
      if (await this.applyEffect(plugin.manifest, event, effect)) accepted.push(effect);
    }
    return accepted;
  }

  private async applyEffect(manifest: PluginManifest, event: PluginEvent, effect: PluginEffect): Promise<boolean> {
    switch (effect.kind) {
      case "notice":
        pushToast(effect.message, effect.level === "error" ? "error" : "info");
        return true;
      case "replace-block-text": {
        if (!manifest.capabilities.includes("graph.write.block")) return false;
        if ((event.kind !== "command" && event.kind !== "slash-command") || event.focusedBlock?.id !== effect.blockId) {
          return false;
        }
        const block = doc.byId[effect.blockId];
        if (!block || block.raw !== effect.expectedRaw) return false;
        setRaw(effect.blockId, effect.raw, { timetracking: false });
        return true;
      }
      case "insert-at-caret":
        // The editor owns caret-aware insertion. This effect is returned only to
        // the slash-command bridge, which performs the actual edit synchronously.
        return event.kind === "slash-command" && manifest.capabilities.includes("slash-commands.register");
      case "block-decoration":
        return (
          event.kind === "decorate-blocks" &&
          manifest.capabilities.includes("block-decorations.register") &&
          event.blocks.some((block) => block.id === effect.blockId)
        );
      case "set-setting": {
        if (!manifest.capabilities.includes("settings.write")) return false;
        const definition = manifest.settings?.find((item) => item.key === effect.key);
        if (!definition) return false;
        const current = installedPlugins().find(
          (item) => item.manifest.id === manifest.id && item.manifest.version === manifest.version
        );
        const value = effect.value === null ? definition.default : effect.value;
        if (!settingAccepts(definition, value)) return false;
        await this.storeSettings(manifest, { ...(current?.settings ?? defaultPluginSettings(manifest.settings)), [effect.key]: value }, [effect.key], false);
        return true;
      }
    }
  }

  private patch(id: string, version: string | undefined, values: Partial<ManagedPlugin>) {
    setInstalledPlugins((current) =>
      current.map((plugin) =>
        plugin.manifest.id === id && (version === undefined || plugin.manifest.version === version)
          ? { ...plugin, ...values }
          : plugin
      )
    );
  }

  private settingsStorageKey(id: string): string {
    return `plugin-settings:${id}`;
  }

  private async loadSettings(manifest: PluginManifest): Promise<PluginSettings> {
    const text = await backend().getAppString(this.settingsStorageKey(manifest.id), "{}");
    return parsePluginSettingsBlob(manifest.settings, text);
  }

  private patchSettings(id: string, settings: PluginSettings) {
    setInstalledPlugins((current) => current.map((plugin) =>
      plugin.manifest.id === id
        ? { ...plugin, settings: validatePluginSettings(plugin.manifest.settings, settings) }
        : plugin
    ));
  }

  private async storeSettings(
    manifest: PluginManifest,
    candidate: PluginSettings,
    changedKeys: string[],
    notifyRunning: boolean
  ) {
    const settings = validatePluginSettings(manifest.settings, candidate);
    await backend().setAppString(this.settingsStorageKey(manifest.id), JSON.stringify(settings));
    this.patchSettings(manifest.id, settings);
    const active = this.active.get(manifest.id);
    if (notifyRunning && active?.manifest.version === manifest.version && manifest.capabilities.includes("settings.read")) {
      await this.invokeAndApply(active, {
        protocolVersion: PLUGIN_PROTOCOL_VERSION,
        kind: "settings-changed",
        settings,
        changedKeys,
      });
    }
  }
}

export const pluginManager = new PluginManager();
