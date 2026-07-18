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
  capturePluginGraphOwner,
  isPluginGraphOwnerCurrent,
  type OwnedPluginBlockSnapshot,
  type PluginGraphOwner,
} from "./ownership";
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

type InvocationAuthority = {
  plugin: ActivePlugin;
  phase: "active" | "starting";
  graphOwner?: PluginGraphOwner;
};

export interface OwnedPluginBlockList {
  owner: PluginGraphOwner;
  blocks: PluginBlockSnapshot[];
}

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
  private readonly starting = new Map<string, { version: string; runtime: PluginRuntime }>();
  private platform: PluginPlatform = "desktop";
  private revoked: RevokedPluginVersions = new Set();
  private activationHeld = false;
  private initialized = false;
  private readonly durableDisablePending = new Set<string>();
  private readonly desiredEnabled = new Map<string, boolean>();
  private readonly intentGeneration = new Map<string, number>();
  private readonly persistenceChains = new Map<string, Promise<void>>();

  private recordIntent(key: string, enabled: boolean): number {
    const generation = (this.intentGeneration.get(key) ?? 0) + 1;
    this.intentGeneration.set(key, generation);
    this.desiredEnabled.set(key, enabled);
    return generation;
  }

  private intentIsCurrent(key: string, generation: number, enabled: boolean): boolean {
    return this.intentGeneration.get(key) === generation && this.desiredEnabled.get(key) === enabled;
  }

  private async enqueuePersistence<T>(key: string, operation: () => Promise<T>): Promise<T> {
    const previous = this.persistenceChains.get(key) ?? Promise.resolve();
    const result = previous.catch(() => {}).then(operation);
    const tail = result.then(() => undefined, () => undefined);
    this.persistenceChains.set(key, tail);
    try {
      return await result;
    } finally {
      if (this.persistenceChains.get(key) === tail) this.persistenceChains.delete(key);
    }
  }

  async initialize(revoked: RevokedPluginVersions = new Set(), activationHeld = false) {
    // Seed before the first await. A live refresh may supersede this set while
    // platform/storage reads are pending, but initialization never writes the
    // older startup snapshot again afterward.
    this.revoked = new Set(revoked);
    this.activationHeld = activationHeld;
    this.initialized = false;
    this.desiredEnabled.clear();
    this.intentGeneration.clear();
    this.platform = await platformKind();
    const records = await backend().listInstalledPlugins();
    const parsed = await Promise.all(records.map(async (record) => {
      const plugin = this.parseRecord(record);
      if (!plugin.error) plugin.settings = await this.loadSettings(plugin.manifest);
      const key = versionKey(plugin.manifest.id, plugin.manifest.version);
      const desired = record.enabled && !this.revoked.has(key);
      const intent = this.recordIntent(key, desired);
      return { record, plugin, intent };
    }));
    const managed = parsed.map(({ plugin }) => plugin);
    setInstalledPlugins(managed);
    this.initialized = true;
    for (const { record, plugin, intent } of parsed) {
      const key = versionKey(plugin.manifest.id, plugin.manifest.version);
      if (record.enabled && this.revoked.has(key)) {
        const revokeIntent = this.intentIsCurrent(key, intent, false)
          ? intent
          : this.recordIntent(key, false);
        await this.persistRevokedDisabled(plugin, revokeIntent);
        continue;
      }
      if (plugin.enabled && !this.activationHeld) {
        // A corrupt or incompatible persisted plugin disables only itself. The
        // remaining enabled plugins still get their independent startup attempt.
        try {
          await this.start(plugin.manifest.id, plugin.manifest.version, false, intent);
        } catch {
          // start() records the causal error and persists the disabled state.
        }
      }
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
    const plugin = installedPlugins().find(
      (item) => item.manifest.id === id && item.manifest.version === version
    );
    if (!plugin) throw new Error("plugin version is not installed");
    const key = versionKey(id, version);
    if (this.revoked.has(key)) throw new Error("this plugin version has been revoked");
    const intent = this.recordIntent(key, true);
    if (this.activationHeld) {
      const persisted = await this.persistEnabledIntent(plugin, intent);
      if (persisted && this.intentIsCurrent(key, intent, true)) {
        this.patch(id, version, { enabled: true, running: false, error: undefined });
      }
      return;
    }
    await this.start(id, version, true, intent);
  }

  async setActivationHold(held: boolean): Promise<void> {
    if (this.activationHeld === held) return;
    this.activationHeld = held;
    if (held || !this.initialized) return;
    for (const plugin of installedPlugins()) {
      const key = versionKey(plugin.manifest.id, plugin.manifest.version);
      const intent = this.intentGeneration.get(key) ?? 0;
      if (!plugin.enabled || plugin.running || this.revoked.has(key) || !this.intentIsCurrent(key, intent, true)) continue;
      try {
        await this.start(plugin.manifest.id, plugin.manifest.version, false, intent);
      } catch {
        // start() records the causal failure and prevents other resumptions from
        // being coupled to one broken package.
      }
    }
  }

  async disable(id: string): Promise<void> {
    const current = installedPlugins().find((plugin) => plugin.manifest.id === id && plugin.selected);
    if (!current) {
      this.patch(id, undefined, { enabled: false, running: false, error: undefined });
      return;
    }
    const key = versionKey(current.manifest.id, current.manifest.version);
    const intent = this.recordIntent(key, false);
    this.active.get(id)?.runtime.dispose();
    this.active.delete(id);
    const starting = this.starting.get(id);
    if (starting?.version === current.manifest.version) {
      starting.runtime.dispose();
      this.starting.delete(id);
    }
    await this.persistDisabledIntent(current, intent, undefined);
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

  async invokeCommand(pluginId: string, contributionId: string, focusedBlock?: OwnedPluginBlockSnapshot) {
    const plugin = this.active.get(pluginId);
    if (!plugin) throw new Error("plugin is not running");
    const contribution = plugin.manifest.contributions?.commands?.find((item) => item.id === contributionId);
    if (!contribution || !supportsPlatform(plugin.manifest, this.platform, contribution.platforms)) {
      throw new Error("plugin command is unavailable");
    }
    const graphOwner = focusedBlock?.owner ?? capturePluginGraphOwner();
    if (!graphOwner || !isPluginGraphOwnerCurrent(graphOwner)) return;
    const event: PluginEvent = {
      protocolVersion: PLUGIN_PROTOCOL_VERSION,
      kind: "command",
      contributionId,
      ...(focusedBlock ? { focusedBlock: focusedBlock.block } : {}),
    };
    await this.invokeAndApply({ plugin, phase: "active", graphOwner }, event);
  }

  async invokeSlashCommand(
    pluginId: string,
    contributionId: string,
    focusedBlock: OwnedPluginBlockSnapshot
  ): Promise<PluginEffect[]> {
    const plugin = this.active.get(pluginId);
    if (!plugin) throw new Error("plugin is not running");
    const contribution = plugin.manifest.contributions?.slashCommands?.find((item) => item.id === contributionId);
    if (!contribution || !supportsPlatform(plugin.manifest, this.platform, contribution.platforms)) {
      throw new Error("plugin slash command is unavailable");
    }
    return this.invokeAndApply({ plugin, phase: "active", graphOwner: focusedBlock.owner }, {
      protocolVersion: PLUGIN_PROTOCOL_VERSION,
      kind: "slash-command",
      contributionId,
      focusedBlock: focusedBlock.block,
    });
  }

  async decorateBlocks(pluginId: string, contributionId: string, owned: OwnedPluginBlockList): Promise<PluginEffect[]> {
    const plugin = this.active.get(pluginId);
    if (!plugin) return [];
    if (!isPluginGraphOwnerCurrent(owned.owner)) return [];
    const contribution = plugin.manifest.contributions?.blockDecorations?.find((item) => item.id === contributionId);
    if (!contribution || !supportsPlatform(plugin.manifest, this.platform, contribution.platforms)) return [];
    const event: PluginEvent = {
      protocolVersion: PLUGIN_PROTOCOL_VERSION,
      kind: "decorate-blocks",
      contributionId,
      blocks: owned.blocks,
    };
    return this.invokeAndApply({ plugin, phase: "active", graphOwner: owned.owner }, event);
  }

  async applyRevocations(revoked: RevokedPluginVersions) {
    this.revoked = new Set(revoked);
    const runtimeRevoked = new Set<string>();
    const installed = installedPlugins();
    const disableIntents = new Map<string, { plugin: ManagedPlugin; intent: number }>();
    for (const plugin of installed) {
      const key = versionKey(plugin.manifest.id, plugin.manifest.version);
      if (!this.revoked.has(key)) continue;
      const hasRuntime = this.starting.get(plugin.manifest.id)?.version === plugin.manifest.version
        || this.active.get(plugin.manifest.id)?.manifest.version === plugin.manifest.version;
      if (plugin.enabled || plugin.running || hasRuntime || this.desiredEnabled.get(key) === true
          || this.persistenceChains.has(key) || this.durableDisablePending.has(key)) {
        disableIntents.set(key, { plugin, intent: this.recordIntent(key, false) });
      }
    }
    for (const [id, starting] of this.starting) {
      if (this.revoked.has(versionKey(id, starting.version))) {
        // A live revocation can arrive while entry bytes, settings, or guest
        // activation are in flight. Terminate that worker before it can become
        // active; start() observes the revocation and persists the disabled state.
        starting.runtime.dispose();
        this.starting.delete(id);
        runtimeRevoked.add(versionKey(id, starting.version));
      }
    }
    for (const [id, active] of this.active) {
      if (this.revoked.has(versionKey(id, active.manifest.version))) {
        active.runtime.dispose();
        this.active.delete(id);
        runtimeRevoked.add(versionKey(id, active.manifest.version));
      }
    }
    for (const plugin of installed) {
      const key = versionKey(plugin.manifest.id, plugin.manifest.version);
      if (!this.revoked.has(key)) continue;
      const disable = disableIntents.get(key);
      if (disable || runtimeRevoked.has(key)) {
        const intent = disable?.intent ?? this.recordIntent(key, false);
        await this.persistRevokedDisabled(plugin, intent);
      } else {
        this.patch(plugin.manifest.id, plugin.manifest.version, {
          enabled: false,
          running: false,
          error: "This version was revoked by the registry.",
        });
      }
    }
  }

  private async persistEnabledIntent(plugin: ManagedPlugin, intent: number): Promise<boolean> {
    const key = versionKey(plugin.manifest.id, plugin.manifest.version);
    return this.enqueuePersistence(key, async () => {
      if (!this.intentIsCurrent(key, intent, true) || this.revoked.has(key)) return false;
      await backend().setPluginEnabled(plugin.storageId, plugin.storageVersion, true);
      return this.intentIsCurrent(key, intent, true) && !this.revoked.has(key);
    });
  }

  private async persistDisabledIntent(
    plugin: ManagedPlugin,
    intent: number,
    error: string | undefined
  ): Promise<boolean> {
    const key = versionKey(plugin.manifest.id, plugin.manifest.version);
    return this.enqueuePersistence(key, async () => {
      if (!this.intentIsCurrent(key, intent, false)) return false;
      await backend().setPluginEnabled(plugin.storageId, plugin.storageVersion, false);
      if (!this.intentIsCurrent(key, intent, false)) return false;
      this.patch(plugin.manifest.id, plugin.manifest.version, {
        enabled: false,
        running: false,
        error,
      });
      return true;
    });
  }

  private async persistRevokedDisabled(plugin: ManagedPlugin, intent: number): Promise<void> {
    const key = versionKey(plugin.manifest.id, plugin.manifest.version);
    try {
      await this.enqueuePersistence(key, async () => {
        if (!this.intentIsCurrent(key, intent, false)) return;
        await backend().setPluginEnabled(plugin.storageId, plugin.storageVersion, false);
        if (!this.intentIsCurrent(key, intent, false)) return;
        this.durableDisablePending.delete(key);
        this.patch(plugin.manifest.id, plugin.manifest.version, {
          enabled: false,
          running: false,
          error: "This version was revoked by the registry.",
        });
      });
    } catch (error) {
      if (this.intentIsCurrent(key, intent, false)) {
        this.durableDisablePending.add(key);
        this.patch(plugin.manifest.id, plugin.manifest.version, {
          enabled: false,
          running: false,
          error: `This version was revoked by the registry. Could not persist disabled state: ${displayError(error)}`,
        });
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

  private async start(id: string, version: string, persist: boolean, intent: number) {
    const plugin = installedPlugins().find(
      (item) => item.manifest.id === id && item.manifest.version === version
    );
    if (!plugin) throw new Error("plugin version is not installed");
    if (!supportsPlatform(plugin.manifest, this.platform)) throw new Error(`plugin does not support ${this.platform}`);
    const key = versionKey(id, version);
    const assertAllowed = () => {
      if (this.revoked.has(key)) throw new Error("this plugin version has been revoked");
      if (this.activationHeld) throw new Error("plugin activation is held until the signed registry is verified");
      if (!this.intentIsCurrent(key, intent, true)) throw new Error("plugin enablement changed while activation was in progress");
    };
    assertAllowed();
    let runtime: PluginRuntime | undefined;
    let registeredStarting = false;
    try {
      const bytes = await backend().readPluginEntry(id, version);
      assertAllowed();
      if (plugin.sha256 !== "mock" && (await digestHex(bytes)) !== plugin.sha256) {
        throw new Error("installed plugin digest does not match its recorded bytes");
      }
      assertAllowed();
      runtime = await PluginRuntime.create(bytesBuffer(bytes));
      assertAllowed();
      this.starting.set(id, { version, runtime });
      registeredStarting = true;
      const assertStarting = () => {
        assertAllowed();
        const owner = this.starting.get(id);
        if (owner?.version !== version || owner.runtime !== runtime) {
          throw new Error("plugin startup was superseded");
        }
      };
      const settings = await this.loadSettings(plugin.manifest);
      assertStarting();
      this.patchSettings(id, settings);
      const active: ActivePlugin = { manifest: plugin.manifest, runtime };
      await this.invokeAndApply({ plugin: active, phase: "starting" }, {
        protocolVersion: PLUGIN_PROTOCOL_VERSION,
        kind: "activate",
        platform: this.platform,
        capabilities: plugin.manifest.capabilities,
        settings: plugin.manifest.capabilities.includes("settings.read") ? settings : {},
      });
      assertStarting();
      if (persist) {
        await this.persistEnabledIntent(plugin, intent);
        assertStarting();
      }
      assertStarting();
      this.active.get(id)?.runtime.dispose();
      this.starting.delete(id);
      this.active.set(id, active);
      runtime = undefined;
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
      const ownsStarting = !!runtime && this.starting.get(id)?.runtime === runtime;
      const currentActive = this.active.get(id);
      const hasActiveSuccessor = !!currentActive && currentActive.runtime !== runtime;
      if (runtime) {
        if (ownsStarting) this.starting.delete(id);
        runtime.dispose();
      }
      // Only the exact starting attempt (or a failure before it registered a
      // worker) may persist/paint its failure. A superseded attempt cannot
      // disable another starting or already-active runtime.
      if (
        (!registeredStarting || ownsStarting)
        && !hasActiveSuccessor
        && this.intentIsCurrent(key, intent, true)
        && !this.revoked.has(key)
      ) {
        const disableIntent = this.recordIntent(key, false);
        await this.persistDisabledIntent(plugin, disableIntent, displayError(error));
      }
      throw error;
    }
  }

  private runtimeAuthorityCurrent(authority: InvocationAuthority): boolean {
    const { plugin } = authority;
    if (this.revoked.has(versionKey(plugin.manifest.id, plugin.manifest.version))) return false;
    if (authority.phase === "active") return this.active.get(plugin.manifest.id) === plugin;
    const starting = this.starting.get(plugin.manifest.id);
    return starting?.version === plugin.manifest.version && starting.runtime === plugin.runtime;
  }

  private invocationAuthorityCurrent(authority: InvocationAuthority): boolean {
    return this.runtimeAuthorityCurrent(authority)
      && (!authority.graphOwner || isPluginGraphOwnerCurrent(authority.graphOwner));
  }

  private async retireFailedActive(authority: InvocationAuthority, error: unknown): Promise<void> {
    if (authority.phase !== "active" || !this.runtimeAuthorityCurrent(authority)) return;
    const { plugin } = authority;
    plugin.runtime.dispose();
    this.active.delete(plugin.manifest.id);
    const managed = installedPlugins().find((candidate) =>
      candidate.manifest.id === plugin.manifest.id
      && candidate.manifest.version === plugin.manifest.version
    );
    if (!managed) return;
    const key = versionKey(plugin.manifest.id, plugin.manifest.version);
    const disableIntent = this.recordIntent(key, false);
    await this.persistDisabledIntent(managed, disableIntent, displayError(error));
  }

  private async invokeAndApply(authority: InvocationAuthority, event: PluginEvent): Promise<PluginEffect[]> {
    if (!this.invocationAuthorityCurrent(authority)) return [];
    let effects: PluginEffect[];
    try {
      effects = (await authority.plugin.runtime.invoke(event)).effects;
    } catch (error) {
      // A genuine timeout/crash has already killed this worker. Retire only the
      // exact still-current active runtime; a starting runtime is owned by start()'s
      // failure path, and a replaced/revoked runtime may not touch its successor.
      await this.retireFailedActive(authority, error);
      throw error;
    }
    if (!this.invocationAuthorityCurrent(authority)) return [];
    const accepted: PluginEffect[] = [];
    for (const effect of effects) {
      if (!this.invocationAuthorityCurrent(authority)) break;
      if (await this.applyEffect(authority, event, effect)) {
        if (!this.invocationAuthorityCurrent(authority)) break;
        accepted.push(effect);
      }
    }
    return this.invocationAuthorityCurrent(authority) ? accepted : [];
  }

  private async applyEffect(authority: InvocationAuthority, event: PluginEvent, effect: PluginEffect): Promise<boolean> {
    if (!this.invocationAuthorityCurrent(authority)) return false;
    const manifest = authority.plugin.manifest;
    switch (effect.kind) {
      case "notice":
        pushToast(effect.message, effect.level === "error" ? "error" : "info");
        return true;
      case "replace-block-text": {
        if (!manifest.capabilities.includes("graph.write.block")) return false;
        if ((event.kind !== "command" && event.kind !== "slash-command") || event.focusedBlock?.id !== effect.blockId) {
          return false;
        }
        if (!this.invocationAuthorityCurrent(authority)) return false;
        const block = doc.byId[effect.blockId];
        if (!block || block.raw !== effect.expectedRaw) return false;
        if (!this.invocationAuthorityCurrent(authority)) return false;
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
      await this.invokeAndApply({ plugin: active, phase: "active" }, {
        protocolVersion: PLUGIN_PROTOCOL_VERSION,
        kind: "settings-changed",
        settings,
        changedKeys,
      });
    }
  }
}

export const pluginManager = new PluginManager();
