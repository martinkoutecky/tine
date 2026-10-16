import { Rich } from "../i18n/Rich";
import { msg, t } from "../i18n";
import { For, Show, createSignal, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { ownedWhen, readOwned } from "../owned";
import { COMMUNITY_REGISTRY_ENABLED, communityThemes, installCommunityTheme } from "../plugins/registry";
import type { GalleryTheme } from "../styles/themes";
import {
  applyThemeColors,
  applyThemeStyle,
  clearThemeSelection,
  galleryThemes,
  selectedThemeColors,
  selectedThemeStyle,
} from "../themeGallery";
import {
  installThemePackage,
  installedThemes,
  themeVersionIsRevoked,
  uninstallThemePackage,
} from "../themes/manager";
import { pushToast } from "../toasts";
import { theme } from "../ui";

// Settings → Appearance theme section: an independent Style (presentation)
// selector and Color scheme gallery (master 670cf75bb), plus theme packages.

type InstalledTheme = ReturnType<typeof installedThemes>[number];
const hasPresentation = (installed: InstalledTheme): boolean =>
  Object.keys(installed.manifest.presentation ?? {}).length > 0;

function galleryBadge(theme: GalleryTheme): string {
  if (theme.modes.length === 1) return theme.modes[0] === "light" ? t("themes-light-only") : t("themes-dark-only");
  return theme.compat === "full" ? t("themes-full") : t("themes-partial");
}

function ThemeGalleryCard(props: {
  id: string;
  name: string;
  author: string;
  badge: string;
  thumbnail: string;
  selected: boolean;
}): JSX.Element {
  return (
    <button
      class="theme-gallery-card"
      classList={{ selected: props.selected }}
      aria-pressed={props.selected}
      onClick={() => applyThemeColors(props.id)}
    >
      <span class="theme-gallery-thumb">
        <img src={props.thumbnail} alt="" loading="lazy" />
      </span>
      <span class="theme-gallery-card-body">
        <span class="theme-gallery-card-top">
          <span class="theme-gallery-name">{props.name}</span>
          <span class="theme-gallery-badge">{props.badge}</span>
        </span>
        <span class="theme-gallery-author">{props.author}</span>
      </span>
    </button>
  );
}

export function ThemeSettings(): JSX.Element {
  let alive = true;
  onCleanup(() => { alive = false; });
  let themePackageInput: HTMLInputElement | undefined;
  const [themePackageBusy, setThemePackageBusy] = createSignal<string | null>(null);
  const installThemeFile = async (files: FileList | null) => {
    const file = files?.[0];
    if (!file) return;
    if (file.size > 64 * 1024) {
      pushToast(msg("themes-theme-manifest-exceeds-the-64-kib-limit"), "error");
      return;
    }
    setThemePackageBusy("install");
    try {
      const installed = await installThemePackage(JSON.parse(await file.text()));
      pushToast(msg("themes-installed", { name: installed.manifest.name, version: installed.manifest.version }), "info");
    } catch (error) {
      pushToast(msg("themes-theme-installation-failed", { error: String(error) }), "error");
    } finally {
      setThemePackageBusy(null);
      if (themePackageInput) themePackageInput.value = "";
    }
  };
  const uninstallTheme = async (key: string, name: string) => {
    const result = await readOwned(ownedWhen(() => alive), backend().confirm(
      t("themes-uninstall-this-removes-the-theme-from", { name: name }),
      t("themes-uninstall-theme")
    ));
    if (result.kind === "stale") return;
    const confirmed = result.value;
    if (!confirmed) return;
    setThemePackageBusy(key);
    try {
      clearThemeSelection(key);
      await uninstallThemePackage(key);
      pushToast(msg("themes-was-uninstalled", { name: name }), "info");
    } catch (error) {
      pushToast(msg("themes-theme-could-not-be-uninstalled", { error: String(error) }), "error");
    } finally {
      setThemePackageBusy(null);
    }
  };
  const installRegistryTheme = async (themeEntry: ReturnType<typeof communityThemes>[number]) => {
    const version = themeEntry.versions[themeEntry.versions.length - 1];
    setThemePackageBusy(`${themeEntry.id}@${version.version}`);
    try {
      const installed = await installCommunityTheme(themeEntry, version);
      pushToast(msg("themes-installed", { name: installed.manifest.name, version: installed.manifest.version }), "info");
    } catch (error) {
      pushToast(msg("themes-community-theme-installation-failed", { error: String(error) }), "error");
    } finally {
      setThemePackageBusy(null);
    }
  };

  return (
    <><div class="settings-section">{t("themes-themes")}</div>
      <div class="settings-row">
        <div><div class="settings-label">{t("themes-style")}</div><div class="settings-hint">{t("themes-typography-journal-headings-and-other-presentation-choices")}</div>
        </div>
        <select
          class="settings-select"
          aria-label={t("themes-theme-style")}
          value={selectedThemeStyle()}
          onChange={(event) => applyThemeStyle(event.currentTarget.value)}
        ><option value="">{t("themes-default")}</option>
          <For each={installedThemes().filter((installed) => !themeVersionIsRevoked(installed.key) && hasPresentation(installed))}>
            {(installed) => <option value={installed.key}>{installed.manifest.name}</option>}
          </For>
        </select>
      </div><div class="settings-section">{t("themes-color-scheme")}</div>
      <div class="theme-gallery-grid">
        <ThemeGalleryCard
          id=""
          name={t("themes-default")}
          author="Tine"
          badge={t("themes-stock")}
          thumbnail="/theme-thumbnails/default.png"
          selected={selectedThemeColors() === ""}
        />
        <For each={galleryThemes}>
          {(theme) => (
            <ThemeGalleryCard
              id={theme.id}
              name={theme.name}
              author={theme.author}
              badge={galleryBadge(theme)}
              thumbnail={theme.thumbnail}
              selected={selectedThemeColors() === theme.id}
            />
          )}
        </For>
      </div>
      <div class="settings-hint theme-gallery-hint"><Rich message={msg("themes-style-and-colors-are-independent-theme-packages")} slots={{ "code": (children) => <code>{children}</code> }} /></div>

      <Show when={COMMUNITY_REGISTRY_ENABLED}><div class="settings-section">{t("themes-theme-packages")}</div>
      <Show when={communityThemes().length > 0}><div class="settings-hint theme-gallery-hint">{t("themes-signed-community-themes-inert-token-manifests")}</div>
        <For each={communityThemes()}>
          {(themeEntry) => {
            const version = () => themeEntry.versions[themeEntry.versions.length - 1];
            const key = () => `${themeEntry.id}@${version().version}`;
            const installed = () => installedThemes().some((theme) => theme.key === key());
            const revoked = () => themeVersionIsRevoked(key());
            return (
              <div class="settings-field">
                <div class="settings-field-row">
                  <div>
                    <div class="settings-label">{themeEntry.name} <span class="settings-hint">v{version().version}</span></div>
                    <div class="settings-hint settings-field-hint">{themeEntry.description}<br />{themeEntry.license}{" · "}{version().modes.join(" + ")}{" · "}{revoked() ? t("themes-revoked-by-the-signed-registry") : version().audit.manualApproval ? t("themes-human-reviewed") : t("themes-low-risk-automated-pass")}
                    </div>
                  </div>
                  <div class="settings-field-control"><button class="settings-link" onClick={() => void backend().openExternal(themeEntry.source)}>{t("themes-details-screenshots")}</button>
                    <button
                      class="settings-btn"
                      disabled={installed() || revoked() || themePackageBusy() !== null || version().audit.status !== "passed"}
                      onClick={() => void installRegistryTheme(themeEntry)}
                    >{revoked() ? t("themes-revoked") : installed() ? t("themes-installed-status") : themePackageBusy() === key() ? t("themes-verifying") : t("themes-install")}
                    </button>
                  </div>
                </div>
              </div>
            );
          }}
        </For>
      </Show>
      </Show>
      <div class="settings-row">
        <div><div class="settings-label">{t("themes-install-a-token-theme")}</div><div class="settings-hint">{t("themes-theme-packages-contain-only-whitelisted-color-tokens")}</div>
        </div>
        <div>
          <input
            ref={themePackageInput}
            type="file"
            accept="application/json,.json"
            style={{ display: "none" }}
            onChange={(event) => void installThemeFile(event.currentTarget.files)}
          />
          <button class="settings-btn" disabled={themePackageBusy() !== null} onClick={() => themePackageInput?.click()}>{themePackageBusy() === "install" ? t("themes-validating") : t("themes-choose-theme-json")}
          </button>
        </div>
      </div><Show when={installedThemes().length > 0} fallback={<p class="settings-hint">{t("themes-no-theme-packages-installed")}</p>}>
        <div class="installed-theme-list">
          <For each={installedThemes()}>
            {(installed) => {
              const previewMode = () => installed.manifest.modes[theme()] ?? installed.manifest.modes.light ?? installed.manifest.modes.dark ?? {};
              const revoked = () => themeVersionIsRevoked(installed.key);
              return (
                <div class="settings-field installed-theme-row">
                  <div class="settings-field-row">
                    <div class="installed-theme-identity">
                      <span
                        class="installed-theme-swatch"
                        aria-hidden="true"
                        style={{
                          background: previewMode()["--ls-primary-background-color"] ?? "var(--bg-secondary)",
                          color: previewMode()["--ls-active-primary-color"] ?? "var(--accent)",
                        }}
                      >●</span>
                      <div>
                        <div class="settings-label">{installed.manifest.name} <span class="settings-hint">v{installed.manifest.version}</span></div><div class="settings-hint">{installed.manifest.author}{" · "}{installed.manifest.license}{" · "}{Object.keys(installed.manifest.modes).join(" + ")}{revoked() ? t("themes-revoked-and-disabled") : ""}</div>
                      </div>
                    </div>
                    <div class="settings-field-control">
                      <button
                        class="settings-btn"
                        disabled={revoked() || selectedThemeColors() === installed.key}
                        onClick={() => applyThemeColors(installed.key)}
                      >{revoked() ? t("themes-revoked") : selectedThemeColors() === installed.key ? t("themes-colors-selected") : t("themes-use-colors")}
                      </button>
                      <Show when={hasPresentation(installed)}>
                        <button
                          class="settings-btn"
                          disabled={revoked() || selectedThemeStyle() === installed.key}
                          onClick={() => applyThemeStyle(installed.key)}
                        >{revoked() ? t("themes-revoked") : selectedThemeStyle() === installed.key ? t("themes-style-selected") : t("themes-use-style")}
                        </button>
                      </Show><button class="settings-link" onClick={() => void backend().openExternal(installed.manifest.source)}>{t("themes-details")}</button>
                      <button
                        class="settings-btn settings-btn-danger"
                        disabled={themePackageBusy() !== null}
                        onClick={() => void uninstallTheme(installed.key, installed.manifest.name)}
                      >{themePackageBusy() === installed.key ? t("themes-uninstalling") : t("themes-uninstall")}
                      </button>
                    </div>
                  </div>
                  <div class="settings-hint settings-field-hint">{installed.manifest.description}</div>
                  <Show when={installed.manifest.portedFrom} keyed>{(origin) => <div class="settings-hint">{t("themes-behavioral-port-of-for-credited-to", { name: origin.name, ecosystem: origin.ecosystem, authors: origin.authors.join(", ") })}</div>}
                  </Show>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </>
  );
}
