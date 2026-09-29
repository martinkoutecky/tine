// Plugins tab of Settings: the installed/community plugin lists, detail page and
// per-plugin settings form. Split out of Settings.tsx (2072 lines, over the og size
// ratchet's growth line) so the busy-ownership fix below is not another edit to the
// oversized file.
import { For, Show, createResource, createSignal, onCleanup, type JSX } from "solid-js";
import { pushToast } from "../toasts";
import { platformKind } from "../platform";
import { Toggle } from "./settingsField";
import { backend } from "../backend";
import { installedPlugins, pluginManager, type ManagedPlugin } from "../plugins/manager";
import { PLUGIN_MANIFEST_MAX_BYTES, PLUGIN_WASM_MAX_BYTES } from "../plugins/manifest";
import {
  COMMUNITY_REGISTRY_ENABLED,
  communityPlugins,
  installCommunityPlugin,
  loadSafetyReport,
  refreshCommunityRegistry,
  registryPersistenceError,
  registryState,
  type PluginSafetyReport,
  type RegistryPlugin,
  type RegistryVersion,
} from "../plugins/registry";
import { ownedWhen, readOwned } from "../owned";
import { readOr } from "../resourceRead";

function PluginSettingsForm(props: {
  plugin: ManagedPlugin;
  busy: () => string | null;
  setBusy: (value: string | null) => void;
}): JSX.Element {
  const operationKey = () => `${props.plugin.manifest.id}@${props.plugin.manifest.version}:settings`;
  const update = async (key: string, value: string | number | boolean) => {
    props.setBusy(operationKey());
    try {
      await pluginManager.setSetting(props.plugin.manifest.id, props.plugin.manifest.version, key, value);
    } catch (error) {
      pushToast(`Plugin setting could not be saved: ${String(error)}`, "error");
    } finally {
      props.setBusy(null);
    }
  };
  const reset = async (key?: string) => {
    props.setBusy(operationKey());
    try {
      if (key) await pluginManager.resetSetting(props.plugin.manifest.id, props.plugin.manifest.version, key);
      else await pluginManager.resetSettings(props.plugin.manifest.id, props.plugin.manifest.version);
    } catch (error) {
      pushToast(`Plugin settings could not be reset: ${String(error)}`, "error");
    } finally {
      props.setBusy(null);
    }
  };

  return (
    <Show
      when={(props.plugin.manifest.settings?.length ?? 0) > 0}
      fallback={<p class="settings-hint">This plugin has no configurable settings.</p>}
    >
      <div class="plugin-settings-list">
        <For each={props.plugin.manifest.settings ?? []}>
          {(definition) => {
            const value = () => props.plugin.settings[definition.key] ?? definition.default;
            const changed = () => value() !== definition.default;
            return (
              <div class="settings-field" data-setting-label={definition.label}>
                <div class="settings-field-row">
                  <div>
                    <div class="settings-label">{definition.label}</div>
                    <div class="settings-hint settings-field-hint">{definition.description}</div>
                  </div>
                  <div class="settings-field-control plugin-setting-control">
                    <Show when={definition.type === "boolean"}>
                      <Toggle
                        on={value() === true}
                        disabled={props.busy() !== null}
                        onClick={() => void update(definition.key, value() !== true)}
                      />
                    </Show>
                    <Show when={definition.type === "enum" && definition.type === "enum"}>
                      <select
                        class="settings-input"
                        aria-label={definition.label}
                        disabled={props.busy() !== null}
                        value={String(value())}
                        onChange={(event) => void update(definition.key, event.currentTarget.value)}
                      >
                        <For each={definition.type === "enum" ? definition.choices : []}>
                          {(choice) => <option value={choice.value}>{choice.label}</option>}
                        </For>
                      </select>
                    </Show>
                    <Show when={definition.type === "number" && definition.type === "number"}>
                      <input
                        class="settings-input plugin-setting-number"
                        type="number"
                        aria-label={definition.label}
                        disabled={props.busy() !== null}
                        value={Number(value())}
                        min={definition.type === "number" ? definition.min : undefined}
                        max={definition.type === "number" ? definition.max : undefined}
                        step={definition.type === "number" ? definition.step ?? "any" : undefined}
                        onChange={(event) => {
                          if (Number.isFinite(event.currentTarget.valueAsNumber)) {
                            void update(definition.key, event.currentTarget.valueAsNumber);
                          }
                        }}
                      />
                    </Show>
                    <Show when={definition.type === "string" && definition.type === "string"}>
                      <input
                        class="settings-input"
                        type="text"
                        aria-label={definition.label}
                        disabled={props.busy() !== null}
                        value={String(value())}
                        maxLength={definition.type === "string" ? definition.maxLength : undefined}
                        onChange={(event) => void update(definition.key, event.currentTarget.value)}
                      />
                    </Show>
                    <Show when={changed()}>
                      <button class="settings-link" disabled={props.busy() !== null} onClick={() => void reset(definition.key)}>
                        Reset
                      </button>
                    </Show>
                  </div>
                </div>
              </div>
            );
          }}
        </For>
      </div>
      <button class="settings-btn" disabled={props.busy() !== null} onClick={() => void reset()}>
        Reset all settings
      </button>
      <p class="settings-hint">Stored on this device only. Plugin settings are never written into your graph.</p>
    </Show>
  );
}

export function PluginsTab(): JSX.Element {
  let alive = true;
  onCleanup(() => { alive = false; });
  let packageInput: HTMLInputElement | undefined;
  const [busy, setBusy] = createSignal<string | null>(null);
  const [view, setView] = createSignal<"browse" | "installed">("browse");
  const [selectedPluginKey, setSelectedPluginKey] = createSignal<string | null>(null);
  const [currentPlatformResource] = createResource(platformKind);
  // `platformKind` is uncaught here; an unknown platform reads as "not yet known",
  // which is what the buttons below already render.
  const currentPlatform = () => readOr(currentPlatformResource, undefined, "plugin platform");
  const selectedPlugin = () => {
    const key = selectedPluginKey();
    return key ? installedPlugins().find((plugin) => `${plugin.manifest.id}@${plugin.manifest.version}` === key) : undefined;
  };

  const installFiles = async (files: FileList | null) => {
    if (!files?.length) return;
    const selected = Array.from(files);
    const manifestFile = selected.find((file) => file.name === "manifest.json") ?? selected.find((file) => file.name.endsWith(".json"));
    const wasmFile = selected.find((file) => file.name.endsWith(".wasm"));
    if (!manifestFile || !wasmFile) {
      pushToast("Choose both manifest.json and the plugin's .wasm entry.", "error");
      return;
    }
    if (manifestFile.size > PLUGIN_MANIFEST_MAX_BYTES || wasmFile.size > PLUGIN_WASM_MAX_BYTES) {
      pushToast("Plugin package is too large (manifest ≤ 64 KiB, .wasm ≤ 8 MiB).", "error");
      return;
    }
    setBusy("install");
    try {
      const manifest: unknown = JSON.parse(await manifestFile.text());
      const plugin = await pluginManager.install(manifest, new Uint8Array(await wasmFile.arrayBuffer()));
      pushToast(`${plugin.manifest.name} ${plugin.manifest.version} installed disabled. Review it, then enable it here.`, "info");
      setView("installed");
      setSelectedPluginKey(`${plugin.manifest.id}@${plugin.manifest.version}`);
    } catch (error) {
      pushToast(`Plugin installation failed: ${String(error)}`, "error");
    } finally {
      setBusy(null);
      if (packageInput) packageInput.value = "";
    }
  };

  const togglePlugin = async (id: string, version: string, enabled: boolean) => {
    setBusy(`${id}@${version}`);
    try {
      if (enabled) await pluginManager.disable(id);
      else await pluginManager.enable(id, version);
    } catch (error) {
      pushToast(`Plugin could not be ${enabled ? "disabled" : "enabled"}: ${String(error)}`, "error");
    } finally {
      setBusy(null);
    }
  };

  const uninstallPlugin = async (plugin: ReturnType<typeof installedPlugins>[number]) => {
    const { id, name, version } = plugin.manifest;
    const result = await readOwned(ownedWhen(() => alive), backend().confirm(
      `Uninstall ${name} ${version}?\n\nThis removes the plugin from this device. It does not change your graph or notes.`,
      "Uninstall plugin?"
    ));
    if (result.kind === "stale") return;
    if (!result.value) return;
    setBusy(`${id}@${version}:uninstall`);
    try {
      await pluginManager.uninstall(id, version);
      pushToast(`${name} ${version} was uninstalled.`, "info");
      if (selectedPluginKey() === `${id}@${version}`) setSelectedPluginKey(null);
    } catch (error) {
      pushToast(`Plugin could not be uninstalled: ${String(error)}`, "error");
    } finally {
      setBusy(null);
    }
  };

  const findingSeverityLabel = (severity: PluginSafetyReport["findings"][number]["severity"]): string => {
    if (severity === "info") return "Information";
    return `${severity[0].toUpperCase()}${severity.slice(1)}-risk finding`;
  };

  const installCommunity = async (plugin: RegistryPlugin, version: RegistryVersion) => {
    setBusy(`${plugin.id}@${version.version}`);
    try {
      const installed = await installCommunityPlugin(plugin, version);
      pushToast(`${installed.manifest.name} installed disabled. Enable it after reviewing its capabilities.`, "info");
      setView("installed");
      setSelectedPluginKey(`${installed.manifest.id}@${installed.manifest.version}`);
    } catch (error) {
      pushToast(`Community plugin installation failed: ${String(error)}`, "error");
    } finally {
      setBusy(null);
    }
  };

  return (
    <>
      <Show when={selectedPlugin()} keyed>
        {(plugin) => (
          <div class="plugin-detail-page">
            <button class="settings-link plugin-detail-back" onClick={() => setSelectedPluginKey(null)}>← Installed plugins</button>
            <div class="plugin-detail-heading">
              <div>
                <h2>{plugin.manifest.name}</h2>
                <div class="settings-hint"><code>{plugin.manifest.id}</code> · v{plugin.manifest.version}</div>
              </div>
              <Toggle
                on={plugin.enabled && plugin.running}
                disabled={busy() !== null}
                onClick={() => void togglePlugin(plugin.manifest.id, plugin.manifest.version, plugin.enabled)}
              />
            </div>
            <p>{plugin.manifest.description}</p>
            <div class="settings-hint">
              {plugin.manifest.author} · {plugin.manifest.license} · {plugin.manifest.platforms.join(", ")}
              <br />Capabilities: {plugin.manifest.capabilities.length ? plugin.manifest.capabilities.join(", ") : "none"}
            </div>
            <Show when={plugin.manifest.portedFrom} keyed>
              {(origin) => (
                <div class="plugin-origin-card">
                  <strong>{origin.relationship === "behavioral-port" ? "Behavioral port" : "Source-derived port"}</strong>
                  <br /><span>From {origin.name} for {origin.ecosystem}; original authors: {origin.authors.join(", ")}.</span>
                  <br /><button class="settings-link" onClick={() => void backend().openExternal(origin.source)}>Original source at {origin.revision.slice(0, 12)}</button>
                </div>
              )}
            </Show>
            <div class="settings-section">Settings</div>
            <PluginSettingsForm plugin={plugin} busy={busy} setBusy={setBusy} />
            <div class="settings-section">Package</div>
            <div class="plugin-detail-actions">
              <button class="settings-btn" onClick={() => void backend().openExternal(plugin.manifest.source)}>Details &amp; screenshots</button>
              <button
                class="settings-btn settings-btn-danger"
                disabled={busy() !== null}
                onClick={() => void uninstallPlugin(plugin)}
              >
                {busy() === `${plugin.manifest.id}@${plugin.manifest.version}:uninstall` ? "Uninstalling…" : "Uninstall…"}
              </button>
            </div>
            <Show when={plugin.error}>
              <div class="settings-hint" style={{ color: "var(--danger, #c44)" }}>{plugin.error}</div>
            </Show>
          </div>
        )}
      </Show>
      <Show when={!selectedPlugin()}>
        <div class="plugin-settings-nav" role="tablist" aria-label="Plugin settings sections">
          <button role="tab" aria-selected={view() === "browse"} classList={{ active: view() === "browse" }} onClick={() => setView("browse")}>Browse</button>
          <button role="tab" aria-selected={view() === "installed"} classList={{ active: view() === "installed" }} onClick={() => setView("installed")}>Installed ({installedPlugins().length})</button>
        </div>
      <Show when={view() === "browse"}>
      <div class="settings-section">Experimental plugin platform</div>
      <p class="settings-hint">
        Tine plugins are capability-limited WebAssembly guests, not Logseq or Obsidian plugins. They cannot directly
        access the DOM, Tauri, the network, files, processes, or your graph. A plugin version is installed disabled and
        runs only after its declared capabilities and entry validate.
      </p>
      <div class="settings-row">
        <div>
          <div class="settings-label">Install a local package</div>
          <div class="settings-hint">Select its <code>manifest.json</code> and <code>.wasm</code> file together.</div>
        </div>
        <div>
          <input
            ref={packageInput}
            type="file"
            multiple
            accept="application/json,.json,application/wasm,.wasm"
            style={{ display: "none" }}
            onChange={(event) => void installFiles(event.currentTarget.files)}
          />
          <button class="settings-btn" disabled={busy() !== null} onClick={() => packageInput?.click()}>
            {busy() === "install" ? "Validating…" : "Choose package…"}
          </button>
        </div>
      </div>

      <Show when={COMMUNITY_REGISTRY_ENABLED}>
      <div class="settings-section">Community catalogue</div>
      <div class="settings-hint">
        Signed registry · automated deterministic and no-tools AI audits · immutable version digests.
        <Show when={registryState() === "offline"}> Showing the last verified offline copy.</Show>
        <Show when={registryState() === "unsafe"}> Installed plugins are held until a signed catalogue can be verified.</Show>
        <Show when={registryPersistenceError()}> {registryPersistenceError()}</Show>
      </div>
      <Show
        when={communityPlugins().length > 0}
        fallback={
          <div class="settings-row">
            <span class="settings-hint">
              {registryState() === "loading" ? "Checking the signed catalogue…" : "No verified catalogue is available."}
            </span>
            <button class="settings-btn" disabled={registryState() === "loading"} onClick={() => void refreshCommunityRegistry()}>
              Retry
            </button>
          </div>
        }
      >
        <For each={communityPlugins()}>
          {(plugin) => {
            const version = () => plugin.versions[plugin.versions.length - 1];
            const installed = () =>
              installedPlugins().some((item) => item.manifest.id === plugin.id && item.manifest.version === version().version);
            const available = () => {
              const platform = currentPlatform();
              return platform ? version().platforms.includes(platform) : false;
            };
            const [reportOpen, setReportOpen] = createSignal(false);
            const [reportState, setReportState] = createSignal<"idle" | "loading" | "ready" | "error">("idle");
            const [report, setReport] = createSignal<PluginSafetyReport | null>(null);
            const showReport = async () => {
              if (reportOpen()) {
                setReportOpen(false);
                return;
              }
              setReportOpen(true);
              if (report()) return;
              setReportState("loading");
              try {
                setReport(await loadSafetyReport(plugin, version()));
                setReportState("ready");
              } catch {
                setReportState("error");
              }
            };
            const safetyLabel = () =>
              version().audit.manualApproval
                ? "Human-reviewed before publication"
                : version().audit.risk === "low"
                  ? "Low-risk automated pass"
                  : "Automated review passed";
            return (
              <div class="settings-field">
                <div class="settings-field-row">
                  <span class="settings-label">{plugin.name} <span class="settings-hint">v{version().version}</span></span>
                  <button
                    class="settings-btn"
                    disabled={installed() || busy() !== null || version().audit.status !== "passed" || !available()}
                    onClick={() => void installCommunity(plugin, version())}
                  >
                    {installed()
                      ? "Installed"
                      : !currentPlatform()
                        ? "Checking…"
                        : !available()
                          ? `Unavailable on ${currentPlatform()}`
                          : busy() === `${plugin.id}@${version().version}`
                            ? "Verifying…"
                            : "Install"}
                  </button>
                </div>
                <div class="settings-hint settings-field-hint">
                  {plugin.description}<br />
                  {plugin.license} · {plugin.aiDevelopment === "none" ? "Human-written" : `AI-${plugin.aiDevelopment}`} · {version().platforms.join(", ")}
                  <br />Capabilities: {version().capabilities.length ? version().capabilities.join(", ") : "none"}
                  {" · "}<button class="settings-link" onClick={() => void backend().openExternal(plugin.source)}>Details &amp; screenshots</button>
                </div>
                <div class="plugin-safety-row">
                  <span
                    class="plugin-safety-badge"
                    classList={{ manual: version().audit.manualApproval, low: !version().audit.manualApproval }}
                  >
                    {safetyLabel()}
                  </span>
                  <span class="settings-hint">Checked {version().audit.checkedAt.slice(0, 10)}</span>
                  <button class="settings-link" onClick={() => void showReport()}>
                    {reportOpen() ? "Hide safety report" : "Safety report"}
                  </button>
                </div>
                <Show when={reportOpen()}>
                  <div class="plugin-safety-report">
                    <Show when={reportState() === "loading"}>
                      <div class="settings-hint">Verifying the signed report…</div>
                    </Show>
                    <Show when={reportState() === "error"}>
                      <div class="settings-hint" style={{ color: "var(--danger, #c44)" }}>
                        The report could not be fetched and digest-verified.
                      </div>
                    </Show>
                    <Show when={report()} keyed>
                      {(safety) => (
                        <>
                          <p>{safety.summary}</p>
                          <Show when={safety.manualApproval} keyed>
                            {(approval) => (
                              <div class="plugin-safety-manual">
                                <strong>Why human review was required</strong><br />
                                <Show
                                  when={version().capabilities.includes("graph.write.block")}
                                  fallback={<>An automated check found behavior that Tine requires a person to inspect before publication.</>}
                                >
                                  This plugin can edit the focused block when you run its command. Tine holds every graph-writing
                                  plugin for human review, even when the automated checks otherwise pass.
                                </Show>
                                <Show when={plugin.id === "page.tine.query-filter"}>
                                  <br />The audit also caught that an earlier draft could act on the wrong focused block. The
                                  published plugin was narrowed to query table/board blocks and reviewed again.
                                </Show>
                                <br /><span>Signed review record: {approval.note}</span>
                              </div>
                            )}
                          </Show>
                          <Show when={safety.findings.length > 0}>
                            <div class="plugin-safety-findings">
                              <div class="settings-hint">
                                Severity describes possible impact, not reviewer confidence. “Low-risk” means a contained problem
                                unlikely to affect your notes; “Information” is an observation, not a known harm.
                              </div>
                              <For each={safety.findings}>
                                {(finding) => (
                                  <div class="plugin-safety-finding">
                                    <span class={`plugin-finding-severity severity-${finding.severity}`}>{findingSeverityLabel(finding.severity)}</span>
                                    <div><strong>{finding.title}</strong><br /><span>{finding.impact}</span></div>
                                  </div>
                                )}
                              </For>
                            </div>
                          </Show>
                          <div class="settings-hint">
                            Source <code title={safety.sourceCommit}>{safety.sourceCommit.slice(0, 12)}</code>
                            {" · Package "}<code title={version().sha256}>{version().sha256.slice(0, 12)}</code>
                            {" · Report "}<code title={version().audit.sha256}>{version().audit.sha256.slice(0, 12)}</code>
                            {" · "}{safety.areasReviewed.length} areas reviewed
                            {" · "}<button class="settings-link" onClick={() => void backend().openExternal(version().audit.url)}>Raw report</button>
                          </div>
                          <div class="settings-hint">Automated review is evidence, not a guarantee.</div>
                        </>
                      )}
                    </Show>
                  </div>
                </Show>
              </div>
            );
          }}
        </For>
      </Show>
      </Show>

      </Show>

      <Show when={view() === "installed"}>

      <div class="settings-section">Installed</div>
      <Show when={installedPlugins().length > 0} fallback={<p class="settings-hint">No plugins installed.</p>}>
        <For each={installedPlugins()}>
          {(plugin) => (
            <div class="settings-field">
              <div class="settings-field-row">
                <span class="settings-label">
                  {plugin.manifest.name} <span class="settings-hint">v{plugin.manifest.version}</span>
                </span>
                <div class="settings-field-control">
                  <button
                    class="settings-btn"
                    onClick={() => setSelectedPluginKey(`${plugin.manifest.id}@${plugin.manifest.version}`)}
                  >
                    {(plugin.manifest.settings?.length ?? 0) > 0 ? "Settings…" : "Details…"}
                  </button>
                  <Toggle
                    on={plugin.enabled && plugin.running}
                    disabled={busy() !== null}
                    onClick={() => void togglePlugin(plugin.manifest.id, plugin.manifest.version, plugin.enabled)}
                  />
                  <button
                    class="settings-btn settings-btn-danger"
                    disabled={busy() !== null}
                    onClick={() => void uninstallPlugin(plugin)}
                  >
                    {busy() === `${plugin.manifest.id}@${plugin.manifest.version}:uninstall` ? "Uninstalling…" : "Uninstall…"}
                  </button>
                </div>
              </div>
              <div class="settings-hint settings-field-hint">
                {plugin.manifest.description}<br />
                <code>{plugin.manifest.id}</code> · {plugin.manifest.license} · {plugin.manifest.platforms.join(", ")}
                <Show when={plugin.manifest.aiDevelopment && plugin.manifest.aiDevelopment !== "none"}>
                  {" · "}AI-{plugin.manifest.aiDevelopment}
                </Show>
                <br />Capabilities: {plugin.manifest.capabilities.length ? plugin.manifest.capabilities.join(", ") : "none"}
                {" · "}<button class="settings-link" onClick={() => void backend().openExternal(plugin.manifest.source)}>Details &amp; screenshots</button>
              </div>
              <Show when={plugin.error}>
                <div class="settings-hint" style={{ color: "var(--danger, #c44)" }}>{plugin.error}</div>
              </Show>
            </div>
          )}
        </For>
      </Show>
      </Show>
      </Show>
    </>
  );
}
