update-diagnostics = عیب‌یابی
update-download-manually = دانلود دستی
update-install-update = نصب به‌روزرسانی
update-is-available-but-automatic-updates-are = { $product } { $version } آماده است، اما این نسخهٔ آزمایشی ۳۲ بیتی Windows به‌روزرسانی خودکار ندارد. بستهٔ x86 را دستی دانلود کنید.
update-is-available-the-flatpak-cannot-update = { $product } { $version } آماده است. Flatpak نمی‌تواند خودش را به‌روز کند: بستهٔ جدید .flatpak را دانلود کنید و با flatpak install --user دوباره نصب کنید.
update-is-available-the-portable-build-cannot-update = { $product } { $version } آماده است. نسخهٔ قابل‌حمل خودش را به‌روز نمی‌کند: ZIP جدید را دانلود کنید، از Tine خارج شوید و در همان پوشه استخراج کنید. تنظیمات در پوشهٔ tine-data می‌مانند.
update-couldn-t-restart-tine-quit-and-start = Tine دوباره راه‌اندازی نشد ({ $error }). خارج شوید و دوباره اجرا کنید.
update-downloading = در حال دانلود { $product } { $version }…
update-the-update-was-downloaded-but-not-installed = به‌روزرسانی دانلود شد اما نصب نشد، پس تغییرات ذخیره‌نشده باز می‌مانند. هر وقت آماده بودید دوباره «نصب به‌روزرسانی» را انتخاب کنید.
update-is-available-you-re-on = { $product } { $version } آماده است؛ نسخهٔ شما { $current } است.
update-failure-notice = { update-failure-summary } بخش عیب‌یابی مرحلهٔ خطا را نشان می‌دهد؛ برای زنجیرهٔ علت پاک‌سازی‌شده، Tine را با --debug اجرا کنید.
update-failure-summary = { $phase ->
    [relaunch] به‌روزرسانی نصب شد اما Tine دوباره اجرا نشد.
   *[other] به‌روزرسانی در مرحلهٔ { update-failure-stage } اعمال نشد؛ صفحهٔ انتشارها باز می‌شود.
    }
update-failure-stage = { $stage ->
    [manifest_fetch] دریافت مانیفست
    [manifest_parse] تجزیهٔ مانیفست
    [target_selection] انتخاب سکو
    [download] دانلود
    [signature_verification] تأیید امضا
    [install] نصب
   *[relaunch] اجرای دوباره
    }
