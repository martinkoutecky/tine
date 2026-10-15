
# Interface text; variables carry untranslated names, paths or diagnostic details.
plugins-plugin-setting-could-not-be-saved = Plugin setting could not be saved: { $error }
# $error is an untranslated diagnostic from the operating system or backend.
plugins-plugin-settings-could-not-be-reset = Plugin settings could not be reset: { $error }
plugins-this-plugin-has-no-configurable-settings = This plugin has no configurable settings.
plugins-reset = Reset
plugins-reset-all-settings = Reset all settings
plugins-stored-on-this-device-only-plugin-settings = Stored on this device only. Plugin settings are never written into your graph.
plugins-choose-both-manifest-json-and-the-plugin-s-wasm = Choose both manifest.json and the plugin's .wasm entry.
plugins-plugin-package-is-too-large-manifest = Plugin package is too large (manifest ≤ 64 KiB, .wasm ≤ 8 MiB).
plugins-installed-disabled-review-it-then-enable = { $name } { $version } installed disabled. Review it, then enable it here.
# $error is an untranslated diagnostic from the operating system or backend.
plugins-plugin-installation-failed = Plugin installation failed: { $error }
plugins-plugin-could-not-be = { $enabled ->
    [disabled] Plugin could not be disabled: { $error }
   *[enabled] Plugin could not be enabled: { $error }
    }
plugins-uninstall-this-removes-the-plugin-from = Uninstall { $name } { $version }?{ "\u000A" }{ "\u000A" }This removes the plugin from this device. It does not change your graph or notes.
plugins-uninstall-plugin = Uninstall plugin?
plugins-was-uninstalled = { $name } { $version } was uninstalled.
# $error is an untranslated diagnostic from the operating system or backend.
plugins-plugin-could-not-be-uninstalled = Plugin could not be uninstalled: { $error }
plugins-installed-disabled-enable-it-after-reviewing = { $name } installed disabled. Enable it after reviewing its capabilities.
# $error is an untranslated diagnostic from the operating system or backend.
plugins-community-plugin-installation-failed = Community plugin installation failed: { $error }
plugins-installed-plugins = ← Installed plugins
plugins-behavioral-port = Behavioral port
plugins-source-derived-port = Source-derived port
plugins-from-for-original-authors = From { $name } for { $ecosystem }; original authors: { $authors }.
# $revision is an abbreviated source commit hash.
plugins-original-source-at = Original source at { $revision }
plugins-settings = Settings
plugins-package = Package
plugins-details-screenshots = Details & screenshots
plugins-uninstalling = Uninstalling…
plugins-uninstall = Uninstall…
plugins-plugin-settings-sections = Plugin settings sections
plugins-browse = Browse
plugins-installed = Installed ({ $count })
plugins-experimental-plugin-platform = Experimental plugin platform
plugins-tine-plugins-are-capability-limited-webassembly-guests-not = Tine plugins are capability-limited WebAssembly guests, not Logseq or Obsidian plugins. They cannot directly access the DOM, Tauri, the network, files, processes, or your graph. A plugin version is installed disabled and runs only after its declared capabilities and entry validate.
plugins-install-a-local-package = Install a local package
# Keep manifest.json and .wasm literal inside <code>.
plugins-select-its-manifest-json-and-wasm-file-together = Select its <code>manifest.json</code> and <code>.wasm</code> file together.
plugins-validating = Validating…
plugins-choose-package = Choose package…
plugins-community-catalogue = Community catalogue
plugins-signed-registry-automated-deterministic-and-no-tools = Signed registry · automated deterministic and no-tools AI audits · immutable version digests.
plugins-showing-the-last-verified-offline-copy = Showing the last verified offline copy.
plugins-installed-plugins-are-held-until-a-signed = Installed plugins are held until a signed catalogue can be verified.
plugins-checking-the-signed-catalogue = Checking the signed catalogue…
plugins-no-verified-catalogue-is-available = No verified catalogue is available.
plugins-retry = Retry
plugins-installed-status = Installed
plugins-checking = Checking…
plugins-unavailable-on = Unavailable on { $platform }
plugins-verifying = Verifying…
plugins-install = Install

plugins-human-written = Human-written
# $developmentMethod is the package author’s AI-development classification (for example assisted or generated).
plugins-ai = AI-{ $developmentMethod }
# $date is the audit’s ISO date (YYYY-MM-DD).
plugins-checked = Checked { $date }
plugins-hide-safety-report = Hide safety report
plugins-safety-report = Safety report
plugins-verifying-the-signed-report = Verifying the signed report…
plugins-the-report-could-not-be-fetched-and = The report could not be fetched and digest-verified.
plugins-why-human-review-was-required = Why human review was required
plugins-an-automated-check-found-behavior-that-tine = An automated check found behavior that Tine requires a person to inspect before publication.
plugins-this-plugin-can-edit-the-focused-block = This plugin can edit the focused block when you run its command. Tine holds every graph-writing plugin for human review, even when the automated checks otherwise pass.
plugins-the-audit-also-caught-that-an-earlier = The audit also caught that an earlier draft could act on the wrong focused block. The published plugin was narrowed to query table/board blocks and reviewed again.
plugins-signed-review-record = Signed review record: { $note }
plugins-severity-describes-possible-impact-not-reviewer-confidence = Severity describes possible impact, not reviewer confidence. “Low-risk” means a contained problem unlikely to affect your notes; “Information” is an observation, not a known harm.
plugins-raw-report = Raw report
plugins-automated-review-is-evidence-not-a-guarantee = Automated review is evidence, not a guarantee.
plugins-no-plugins-installed = No plugins installed.
plugins-open-settings = Settings…
plugins-details = Details…

plugins-capabilities-list = { $kind ->
    [none] Capabilities: none
   *[some] Capabilities: { $capabilities }
    }
# Digest values and their code slots are immutable audit identifiers, never translated.
# $source, $package and $report are abbreviated hashes. Their named code slots show the full hash in a tooltip; $count is the number of reviewed areas.
plugins-audit-details = { $count ->
    [one] Source <source-code>{ $source }</source-code> · Package <package-code>{ $package }</package-code> · Report <report-code>{ $report }</report-code> · { $count } areas reviewed
   *[other] Source <source-code>{ $source }</source-code> · Package <package-code>{ $package }</package-code> · Report <report-code>{ $report }</report-code> · { $count } areas reviewed
    }

# Remaining interface text (GH #107). Inline code and syntax stay unchanged.
plugins-ui-could-not-cache-verified-plugin-registry = Could not cache verified plugin registry: { $value1 }
plugins-ui-could-not-cache-plugin-safety-report = Could not cache plugin safety report.

# Helper-return and component-prop interface text (GH #107).
plugins-prose-information = Information
plugins-prose-human-reviewed-before-publication = Human-reviewed before publication
plugins-prose-low-risk-automated-pass = Low-risk automated pass
plugins-prose-automated-review-passed = Automated review passed

plugins-prose-risk-finding = { $severity ->
        [low] Low-risk finding
        [medium] Medium-risk finding
        [critical] Critical-risk finding
       *[high] High-risk finding
    }
