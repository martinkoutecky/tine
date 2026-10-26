
# Interface text; variables carry untranslated names, paths or diagnostic details.
update-diagnostics = Diagnostics
update-download-manually = Download manually
update-install-update = Install update

update-is-available-but-automatic-updates-are = { $product } { $version } is available, but automatic updates are not supported by this experimental 32-bit Windows build. Download the x86 package manually.
update-is-available-the-flatpak-cannot-update = { $product } { $version } is available. The Flatpak cannot update itself: download the new .flatpak bundle and install it again with flatpak install --user.
update-is-available-the-portable-build-cannot-update = { $product } { $version } is available. The portable build does not update itself: download the new portable ZIP, quit Tine, and extract it into the same folder. Your settings stay in its tine-data folder.
# $error is an untranslated diagnostic from the operating system or backend.
update-couldn-t-restart-tine-quit-and-start = Couldn't restart Tine ({ $error }). Quit and start it again.
update-downloading = Downloading { $product } { $version }…
update-the-update-was-downloaded-but-not-installed = The update was downloaded but not installed, so your unsaved changes stay open. Choose Install update again when you are ready.
update-is-available-you-re-on = { $product } { $version } is available — you're on { $current }.

update-failure-notice = { update-failure-summary } Diagnostics has the safe failure stage; launch Tine with --debug for the sanitized cause chain.
update-failure-summary = { $phase ->
    [relaunch] The update installed but Tine could not relaunch.
   *[other] Couldn't apply the update during { update-failure-stage } — opening the releases page instead.
    }
update-failure-stage = { $stage ->
    [manifest_fetch] manifest fetch
    [manifest_parse] manifest parsing
    [target_selection] platform selection
    [download] download
    [signature_verification] signature verification
    [install] installation
   *[relaunch] relaunch
    }
