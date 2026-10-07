import { For, Show, createSignal, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { ownedWhen, readOwned } from "../owned";
import { CustomCssSettings } from "./CustomCssSettings";
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
  if (theme.modes.length === 1) return theme.modes[0] === "light" ? "Light-only" : "Dark-only";
  return theme.compat === "full" ? "Full" : "Partial";
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
      pushToast("Theme manifest exceeds the 64 KiB limit.", "error");
      return;
    }
    setThemePackageBusy("install");
    try {
      const installed = await installThemePackage(JSON.parse(await file.text()));
      pushToast(`${installed.manifest.name} ${installed.manifest.version} installed.`, "info");
    } catch (error) {
      pushToast(`Theme installation failed: ${String(error)}`, "error");
    } finally {
      setThemePackageBusy(null);
      if (themePackageInput) themePackageInput.value = "";
    }
  };
  const uninstallTheme = async (key: string, name: string) => {
    const result = await readOwned(ownedWhen(() => alive), backend().confirm(
      `Uninstall ${name}?\n\nThis removes the theme from this device. It does not change your graph or custom.css.`,
      "Uninstall theme?"
    ));
    if (result.kind === "stale") return;
    const confirmed = result.value;
    if (!confirmed) return;
    setThemePackageBusy(key);
    try {
      clearThemeSelection(key);
      await uninstallThemePackage(key);
      pushToast(`${name} was uninstalled.`, "info");
    } catch (error) {
      pushToast(`Theme could not be uninstalled: ${String(error)}`, "error");
    } finally {
      setThemePackageBusy(null);
    }
  };
  const installRegistryTheme = async (themeEntry: ReturnType<typeof communityThemes>[number]) => {
    const version = themeEntry.versions[themeEntry.versions.length - 1];
    setThemePackageBusy(`${themeEntry.id}@${version.version}`);
    try {
      const installed = await installCommunityTheme(themeEntry, version);
      pushToast(`${installed.manifest.name} ${installed.manifest.version} installed.`, "info");
    } catch (error) {
      pushToast(`Community theme installation failed: ${String(error)}`, "error");
    } finally {
      setThemePackageBusy(null);
    }
  };

  return (
    <>
      <div class="settings-section">Themes</div>
      <div class="settings-row">
        <div>
          <div class="settings-label">Style</div>
          <div class="settings-hint">Typography, journal headings, and other presentation choices.</div>
        </div>
        <select
          class="settings-select"
          aria-label="Theme style"
          value={selectedThemeStyle()}
          onChange={(event) => applyThemeStyle(event.currentTarget.value)}
        >
          <option value="">Default</option>
          <For each={installedThemes().filter((installed) => !themeVersionIsRevoked(installed.key) && hasPresentation(installed))}>
            {(installed) => <option value={installed.key}>{installed.manifest.name}</option>}
          </For>
        </select>
      </div>

      <div class="settings-section">Color scheme</div>
      <div class="theme-gallery-grid">
        <ThemeGalleryCard
          id=""
          name="Default"
          author="Tine"
          badge="Stock"
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
      <div class="settings-hint theme-gallery-hint">
        Style and colors are independent. Theme packages use validated colors and Tine-owned presentation styles; your <code>logseq/custom.css</code> still takes priority.
      </div>

      <CustomCssSettings />

      <Show when={COMMUNITY_REGISTRY_ENABLED}>
      <div class="settings-section">Theme packages</div>
      <Show when={communityThemes().length > 0}>
        <div class="settings-hint theme-gallery-hint">Signed community themes · inert token manifests · immutable audit digests.</div>
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
                    <div class="settings-hint settings-field-hint">
                      {themeEntry.description}<br />{themeEntry.license} · {version().modes.join(" + ")} · {revoked() ? "Revoked by the signed registry" : version().audit.manualApproval ? "Human-reviewed" : "Low-risk automated pass"}
                    </div>
                  </div>
                  <div class="settings-field-control">
                    <button class="settings-link" onClick={() => void backend().openExternal(themeEntry.source)}>Details &amp; screenshots</button>
                    <button
                      class="settings-btn"
                      disabled={installed() || revoked() || themePackageBusy() !== null || version().audit.status !== "passed"}
                      onClick={() => void installRegistryTheme(themeEntry)}
                    >
                      {revoked() ? "Revoked" : installed() ? "Installed" : themePackageBusy() === key() ? "Verifying…" : "Install"}
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
        <div>
          <div class="settings-label">Install a token theme</div>
          <div class="settings-hint">Theme packages contain only whitelisted color tokens and metadata—no scripts, selectors, imports, or remote assets.</div>
        </div>
        <div>
          <input
            ref={themePackageInput}
            type="file"
            accept="application/json,.json"
            style={{ display: "none" }}
            onChange={(event) => void installThemeFile(event.currentTarget.files)}
          />
          <button class="settings-btn" disabled={themePackageBusy() !== null} onClick={() => themePackageInput?.click()}>
            {themePackageBusy() === "install" ? "Validating…" : "Choose theme.json…"}
          </button>
        </div>
      </div>
      <Show when={installedThemes().length > 0} fallback={<p class="settings-hint">No theme packages installed.</p>}>
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
                        <div class="settings-label">{installed.manifest.name} <span class="settings-hint">v{installed.manifest.version}</span></div>
                        <div class="settings-hint">{installed.manifest.author} · {installed.manifest.license} · {Object.keys(installed.manifest.modes).join(" + ")}{revoked() ? " · Revoked and disabled" : ""}</div>
                      </div>
                    </div>
                    <div class="settings-field-control">
                      <button
                        class="settings-btn"
                        disabled={revoked() || selectedThemeColors() === installed.key}
                        onClick={() => applyThemeColors(installed.key)}
                      >
                        {revoked() ? "Revoked" : selectedThemeColors() === installed.key ? "Colors selected" : "Use colors"}
                      </button>
                      <Show when={hasPresentation(installed)}>
                        <button
                          class="settings-btn"
                          disabled={revoked() || selectedThemeStyle() === installed.key}
                          onClick={() => applyThemeStyle(installed.key)}
                        >
                          {revoked() ? "Revoked" : selectedThemeStyle() === installed.key ? "Style selected" : "Use style"}
                        </button>
                      </Show>
                      <button class="settings-link" onClick={() => void backend().openExternal(installed.manifest.source)}>Details</button>
                      <button
                        class="settings-btn settings-btn-danger"
                        disabled={themePackageBusy() !== null}
                        onClick={() => void uninstallTheme(installed.key, installed.manifest.name)}
                      >
                        {themePackageBusy() === installed.key ? "Uninstalling…" : "Uninstall…"}
                      </button>
                    </div>
                  </div>
                  <div class="settings-hint settings-field-hint">{installed.manifest.description}</div>
                  <Show when={installed.manifest.portedFrom} keyed>
                    {(origin) => <div class="settings-hint">Behavioral port of {origin.name} for {origin.ecosystem}, credited to {origin.authors.join(", ")}.</div>}
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
