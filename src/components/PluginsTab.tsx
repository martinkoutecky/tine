import { Rich } from "../i18n/Rich";
import { msg, t } from "../i18n";
import { FailureBoundary } from "./FailureBoundary";
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
import { latestOwner, ownedWhen, readOwned } from "../owned";
import { readOr } from "../resourceRead";

/** The tab's one busy slot. `hold(key)` marks it busy and returns the release,
 * which clears the slot only while this is still the newest hold: an earlier
 * operation finishing (or a cancelled confirm) can never clear the flag of a
 * later one still in flight (master cdd0eda4b; OG-I2 O7). O(1). */
function busySlot(): { busy: () => string | null; hold: (key: string) => () => void } {
  const scope = {};
  const [busy, setBusy] = createSignal<string | null>(null);
  return {
    busy,
    hold: (key) => {
      const mine = latestOwner(scope, "busy");
      setBusy(key);
      return () => { if (mine()) setBusy(null); };
    },
  };
}

function PluginSettingsForm(props: {
  plugin: ManagedPlugin;
  busy: () => string | null;
  hold: (key: string) => () => void;
}): JSX.Element {
  const operationKey = () => `${props.plugin.manifest.id}@${props.plugin.manifest.version}:settings`;
  const update = async (key: string, value: string | number | boolean) => {
    const release = props.hold(operationKey());
    try {
      await pluginManager.setSetting(props.plugin.manifest.id, props.plugin.manifest.version, key, value);
    } catch (error) {
      pushToast(msg("plugins-plugin-setting-could-not-be-saved", { error: String(error) }), "error");
    } finally {
      release();
    }
  };
  const reset = async (key?: string) => {
    const release = props.hold(operationKey());
    try {
      if (key) await pluginManager.resetSetting(props.plugin.manifest.id, props.plugin.manifest.version, key);
      else await pluginManager.resetSettings(props.plugin.manifest.id, props.plugin.manifest.version);
    } catch (error) {
      pushToast(msg("plugins-plugin-settings-could-not-be-reset", { error: String(error) }), "error");
    } finally {
      release();
    }
  };

  return (
    <Show
      when={(props.plugin.manifest.settings?.length ?? 0) > 0}
      fallback={<p class="settings-hint">{t("plugins-this-plugin-has-no-configurable-settings")}</p>}
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
                    <Show when={definition.type === "string" || definition.type === "color"}>
                      <input
                        class="settings-input"
                        type="text"
                        aria-label={definition.label}
                        disabled={props.busy() !== null}
                        value={String(value())}
                        maxLength={definition.type === "string" ? definition.maxLength : 100}
                        onChange={(event) => void update(definition.key, event.currentTarget.value)}
                      />
                    </Show>
                    <Show when={definition.type === "color"}>
                      <input
                        type="color"
                        aria-label={definition.label}
                        disabled={props.busy() !== null}
                        value={/^#[0-9a-f]{6}$/i.test(String(value())) ? String(value()) : "#000000"}
                        onChange={event => void update(definition.key, event.currentTarget.value)}
                      />
                    </Show>
                    <Show when={changed()}>
                      <button class="settings-link" disabled={props.busy() !== null} onClick={() => void reset(definition.key)}>{t("plugins-reset")}</button>
                    </Show>
                  </div>
                </div>
              </div>
            );
          }}
        </For>
      </div>
      <button class="settings-btn" disabled={props.busy() !== null} onClick={() => void reset()}>{t("plugins-reset-all-settings")}</button><p class="settings-hint">{t("plugins-stored-on-this-device-only-plugin-settings")}</p>
    </Show>
  );
}

/** Plugin settings rendering fails locally; Retry remounts only this tab. */
export function PluginsTab(): JSX.Element {
  return <FailureBoundary region="Plugins"><PluginsTabContent /></FailureBoundary>;
}

function PluginsTabContent(): JSX.Element {
  let alive = true;
  onCleanup(() => { alive = false; });
  let packageInput: HTMLInputElement | undefined;
  const { busy, hold } = busySlot();
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
      pushToast(msg("plugins-choose-both-manifest-json-and-the-plugin-s-wasm"), "error");
      return;
    }
    if (manifestFile.size > PLUGIN_MANIFEST_MAX_BYTES || wasmFile.size > PLUGIN_WASM_MAX_BYTES) {
      pushToast(msg("plugins-plugin-package-is-too-large-manifest"), "error");
      return;
    }
    const release = hold("install");
    try {
      const manifest: unknown = JSON.parse(await manifestFile.text());
      const plugin = await pluginManager.install(manifest, new Uint8Array(await wasmFile.arrayBuffer()));
      pushToast(msg("plugins-installed-disabled-review-it-then-enable", { name: plugin.manifest.name, version: plugin.manifest.version }), "info");
      setView("installed");
      setSelectedPluginKey(`${plugin.manifest.id}@${plugin.manifest.version}`);
    } catch (error) {
      pushToast(msg("plugins-plugin-installation-failed", { error: String(error) }), "error");
    } finally {
      release();
      if (packageInput) packageInput.value = "";
    }
  };

  const togglePlugin = async (id: string, version: string, enabled: boolean) => {
    const release = hold(`${id}@${version}`);
    try {
      if (enabled) await pluginManager.disable(id);
      else await pluginManager.enable(id, version);
    } catch (error) {
      pushToast(msg("plugins-plugin-could-not-be", { enabled: enabled ? "disabled" : "enabled", error: String(error) }), "error");
    } finally {
      release();
    }
  };

  const uninstallPlugin = async (plugin: ReturnType<typeof installedPlugins>[number]) => {
    const { id, name, version } = plugin.manifest;
    // Held across the confirm too: the other controls stay disabled while it is open.
    const release = hold(`${id}@${version}:uninstall`);
    try {
      const result = await readOwned(ownedWhen(() => alive), backend().confirm(
        t("plugins-uninstall-this-removes-the-plugin-from", { name: name, version: version }),
        t("plugins-uninstall-plugin")
      ));
      if (result.kind === "stale" || !result.value) return;
      await pluginManager.uninstall(id, version);
      pushToast(msg("plugins-was-uninstalled", { name: name, version: version }), "info");
      if (selectedPluginKey() === `${id}@${version}`) setSelectedPluginKey(null);
    } catch (error) {
      pushToast(msg("plugins-plugin-could-not-be-uninstalled", { error: String(error) }), "error");
    } finally {
      release();
    }
  };

  const findingSeverityLabel = (severity: PluginSafetyReport["findings"][number]["severity"]): string => {
    if (severity === "info") return t("plugins-prose-information");
    return t("plugins-prose-risk-finding", { severity });
  };

  const installCommunity = async (plugin: RegistryPlugin, version: RegistryVersion) => {
    const release = hold(`${plugin.id}@${version.version}`);
    try {
      const installed = await installCommunityPlugin(plugin, version);
      pushToast(msg("plugins-installed-disabled-enable-it-after-reviewing", { name: installed.manifest.name }), "info");
      setView("installed");
      setSelectedPluginKey(`${installed.manifest.id}@${installed.manifest.version}`);
    } catch (error) {
      pushToast(msg("plugins-community-plugin-installation-failed", { error: String(error) }), "error");
    } finally {
      release();
    }
  };

  return (
    <>
      <Show when={selectedPlugin()} keyed>
        {(plugin) => (
          <div class="plugin-detail-page"><button class="settings-link plugin-detail-back" onClick={() => setSelectedPluginKey(null)}>{t("plugins-installed-plugins")}</button>
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
              {plugin.manifest.author} · {plugin.manifest.license} · {plugin.manifest.platforms.join(", ")}<br />{t("plugins-capabilities-list", { kind: plugin.manifest.capabilities.length ? "some" : "none", capabilities: plugin.manifest.capabilities.join(", ") })}
            </div>
            <Show when={plugin.manifest.portedFrom} keyed>
              {(origin) => (
                <div class="plugin-origin-card"><strong>{origin.relationship === "behavioral-port" ? t("plugins-behavioral-port") : t("plugins-source-derived-port")}</strong><br /><span>{t("plugins-from-for-original-authors", { name: origin.name, ecosystem: origin.ecosystem, authors: origin.authors.join(", ") })}</span><br /><button class="settings-link" onClick={() => void backend().openExternal(origin.source)}>{t("plugins-original-source-at", { revision: origin.revision.slice(0, 12) })}</button>
                </div>
              )}
            </Show><div class="settings-section">{t("plugins-settings")}</div>
            <PluginSettingsForm plugin={plugin} busy={busy} hold={hold} /><div class="settings-section">{t("plugins-package")}</div>
            <div class="plugin-detail-actions"><button class="settings-btn" onClick={() => void backend().openExternal(plugin.manifest.source)}>{t("plugins-details-screenshots")}</button>
              <button
                class="settings-btn settings-btn-danger"
                disabled={busy() !== null}
                onClick={() => void uninstallPlugin(plugin)}
              >{busy() === `${plugin.manifest.id}@${plugin.manifest.version}:uninstall` ? t("plugins-uninstalling") : t("plugins-uninstall")}
              </button>
            </div>
            <Show when={plugin.error}>
              <div class="settings-hint" style={{ color: "var(--tine-danger-color, var(--palette-danger))" }}>{plugin.error}</div>
            </Show>
          </div>
        )}
      </Show>
      <Show when={!selectedPlugin()}>
        <div class="plugin-settings-nav" role="tablist" aria-label={t("plugins-plugin-settings-sections")}><button role="tab" aria-selected={view() === "browse"} classList={{ active: view() === "browse" }} onClick={() => setView("browse")}>{t("plugins-browse")}</button><button role="tab" aria-selected={view() === "installed"} classList={{ active: view() === "installed" }} onClick={() => setView("installed")}>{t("plugins-installed", { count: installedPlugins().length })}</button>
        </div>
      <Show when={view() === "browse"}><div class="settings-section">{t("plugins-experimental-plugin-platform")}</div>
      <p class="settings-hint">{t("plugins-tine-plugins-are-capability-limited-webassembly-guests-not")}</p>
      <div class="settings-row">
        <div><div class="settings-label">{t("plugins-install-a-local-package")}</div><div class="settings-hint"><Rich message={msg("plugins-select-its-manifest-json-and-wasm-file-together")} slots={{ "code": (children) => <code>{children}</code> }} /></div>
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
          <button class="settings-btn" disabled={busy() !== null} onClick={() => packageInput?.click()}>{busy() === "install" ? t("plugins-validating") : t("plugins-choose-package")}
          </button>
        </div>
      </div>

      <Show when={COMMUNITY_REGISTRY_ENABLED}><div class="settings-section">{t("plugins-community-catalogue")}</div>
      <div class="settings-hint">{t("plugins-signed-registry-automated-deterministic-and-no-tools")}<Show when={registryState() === "offline"}>{" "}{t("plugins-showing-the-last-verified-offline-copy")}</Show><Show when={registryState() === "unsafe"}>{" "}{t("plugins-installed-plugins-are-held-until-a-signed")}</Show>
        <Show when={registryPersistenceError()}> {registryPersistenceError()}</Show>
      </div>
      <Show
        when={communityPlugins().length > 0}
        fallback={
          <div class="settings-row">
            <span class="settings-hint">{registryState() === "loading" ? t("plugins-checking-the-signed-catalogue") : t("plugins-no-verified-catalogue-is-available")}
            </span>
            <button class="settings-btn" disabled={registryState() === "loading"} onClick={() => void refreshCommunityRegistry()}>{t("plugins-retry")}</button>
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
                ? t("plugins-prose-human-reviewed-before-publication")
                : version().audit.risk === "low"
                  ? t("plugins-prose-low-risk-automated-pass")
                  : t("plugins-prose-automated-review-passed");
            return (
              <div class="settings-field">
                <div class="settings-field-row">
                  <span class="settings-label">{plugin.name} <span class="settings-hint">v{version().version}</span></span>
                  <button
                    class="settings-btn"
                    disabled={installed() || busy() !== null || version().audit.status !== "passed" || !available()}
                    onClick={() => void installCommunity(plugin, version())}
                  >{installed()
                      ? t("plugins-installed-status")
                      : !currentPlatform()
                        ? t("plugins-checking")
                        : !available()
                          ? t("plugins-unavailable-on", { platform: currentPlatform() ?? "" })
                          : busy() === `${plugin.id}@${version().version}`
                            ? t("plugins-verifying")
                            : t("plugins-install")}
                  </button>
                </div>
                <div class="settings-hint settings-field-hint">
                  {plugin.description}<br />{plugin.license}{" · "}{plugin.aiDevelopment === "none" ? t("plugins-human-written") : t("plugins-ai", { developmentMethod: plugin.aiDevelopment })}{" · "}{version().platforms.join(", ")}<br />{t("plugins-capabilities-list", { kind: version().capabilities.length ? "some" : "none", capabilities: version().capabilities.join(", ") })}{" · "}<button class="settings-link" onClick={() => void backend().openExternal(plugin.source)}>{t("plugins-details-screenshots")}</button>
                </div>
                <div class="plugin-safety-row">
                  <span
                    class="plugin-safety-badge"
                    classList={{ manual: version().audit.manualApproval, low: !version().audit.manualApproval }}
                  >
                    {safetyLabel()}
                  </span><span class="settings-hint">{t("plugins-checked", { date: version().audit.checkedAt.slice(0, 10) })}</span>
                  <button class="settings-link" onClick={() => void showReport()}>{reportOpen() ? t("plugins-hide-safety-report") : t("plugins-safety-report")}
                  </button>
                </div>
                <Show when={reportOpen()}>
                  <div class="plugin-safety-report">
                    <Show when={reportState() === "loading"}><div class="settings-hint">{t("plugins-verifying-the-signed-report")}</div>
                    </Show>
                    <Show when={reportState() === "error"}>
                      <div class="settings-hint" style={{ color: "var(--tine-danger-color, var(--palette-danger))" }}>{t("plugins-the-report-could-not-be-fetched-and")}</div>
                    </Show>
                    <Show when={report()} keyed>
                      {(safety) => (
                        <>
                          <p>{safety.summary}</p>
                          <Show when={safety.manualApproval} keyed>
                            {(approval) => (
                              <div class="plugin-safety-manual"><strong>{t("plugins-why-human-review-was-required")}</strong><br />
                                <Show
                                  when={version().capabilities.includes("graph.write.block")}
                                  fallback={<>{t("plugins-an-automated-check-found-behavior-that-tine")}</>}
                                >{t("plugins-this-plugin-can-edit-the-focused-block")}</Show>
                                <Show when={plugin.id === "page.tine.query-filter"}><br />{t("plugins-the-audit-also-caught-that-an-earlier")}</Show><br /><span>{t("plugins-signed-review-record", { note: approval.note })}</span>
                              </div>
                            )}
                          </Show>
                          <Show when={safety.findings.length > 0}>
                            <div class="plugin-safety-findings">
                              <div class="settings-hint">{t("plugins-severity-describes-possible-impact-not-reviewer-confidence")}</div>
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
                          <div class="settings-hint"><Rich message={msg("plugins-audit-details", { source: safety.sourceCommit.slice(0, 12), package: version().sha256.slice(0, 12), report: version().audit.sha256.slice(0, 12), count: safety.areasReviewed.length })} slots={{ "source-code": (children) => <code title={safety.sourceCommit}>{children}</code>, "package-code": (children) => <code title={version().sha256}>{children}</code>, "report-code": (children) => <code title={version().audit.sha256}>{children}</code> }} />{" · "}<button class="settings-link" onClick={() => void backend().openExternal(version().audit.url)}>{t("plugins-raw-report")}</button>
                          </div><div class="settings-hint">{t("plugins-automated-review-is-evidence-not-a-guarantee")}</div>
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

      <Show when={view() === "installed"}><div class="settings-section">{t("plugins-installed-status")}</div><Show when={installedPlugins().length > 0} fallback={<p class="settings-hint">{t("plugins-no-plugins-installed")}</p>}>
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
                  >{(plugin.manifest.settings?.length ?? 0) > 0 ? t("plugins-open-settings") : t("plugins-details")}
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
                  >{busy() === `${plugin.manifest.id}@${plugin.manifest.version}:uninstall` ? t("plugins-uninstalling") : t("plugins-uninstall")}
                  </button>
                </div>
              </div>
              <div class="settings-hint settings-field-hint">
                {plugin.manifest.description}<br />
                <code>{plugin.manifest.id}</code> · {plugin.manifest.license} · {plugin.manifest.platforms.join(", ")}
                <Show when={plugin.manifest.aiDevelopment && plugin.manifest.aiDevelopment !== "none"}>{" · "}{t("plugins-ai", { developmentMethod: plugin.manifest.aiDevelopment ?? "" })}
                </Show><br />{t("plugins-capabilities-list", { kind: plugin.manifest.capabilities.length ? "some" : "none", capabilities: plugin.manifest.capabilities.join(", ") })}{" · "}<button class="settings-link" onClick={() => void backend().openExternal(plugin.manifest.source)}>{t("plugins-details-screenshots")}</button>
              </div>
              <Show when={plugin.error}>
                <div class="settings-hint" style={{ color: "var(--tine-danger-color, var(--palette-danger))" }}>{plugin.error}</div>
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
