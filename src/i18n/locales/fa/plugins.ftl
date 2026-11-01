plugins-plugin-setting-could-not-be-saved = تنظیم افزونه ذخیره نشد: { $error }
plugins-plugin-settings-could-not-be-reset = تنظیمات افزونه بازنشانی نشد: { $error }
plugins-this-plugin-has-no-configurable-settings = این افزونه تنظیم قابل‌پیکربندی ندارد.
plugins-reset = بازنشانی
plugins-reset-all-settings = بازنشانی همهٔ تنظیمات
plugins-stored-on-this-device-only-plugin-settings = فقط روی این دستگاه ذخیره می‌شود. تنظیمات افزونه هرگز در گراف نوشته نمی‌شود.
plugins-choose-both-manifest-json-and-the-plugin-s-wasm = هم manifest.json و هم فایل .wasm افزونه را انتخاب کنید.
plugins-plugin-package-is-too-large-manifest = بستهٔ افزونه بیش از حد بزرگ است (مانیفست ≤ ۶۴ KiB، .wasm ≤ ۸ MiB).
plugins-installed-disabled-review-it-then-enable = { $name } { $version } به‌صورت غیرفعال نصب شد. آن را بررسی و سپس از اینجا فعال کنید.
plugins-plugin-installation-failed = نصب افزونه ناموفق بود: { $error }
plugins-plugin-could-not-be = { $enabled ->
    [disabled] افزونه غیرفعال نشد: { $error }
   *[enabled] افزونه فعال نشد: { $error }
    }
plugins-uninstall-this-removes-the-plugin-from = حذف نصب { $name } { $version }؟{ "\u000A" }{ "\u000A" }این کار افزونه را از این دستگاه حذف می‌کند و به گراف یا یادداشت‌ها دست نمی‌زند.
plugins-uninstall-plugin = حذف نصب افزونه؟
plugins-was-uninstalled = { $name } { $version } حذف نصب شد.
plugins-plugin-could-not-be-uninstalled = افزونه حذف نصب نشد: { $error }
plugins-installed-disabled-enable-it-after-reviewing = { $name } به‌صورت غیرفعال نصب شد. پس از بررسی توانایی‌هایش فعالش کنید.
plugins-community-plugin-installation-failed = نصب افزونهٔ جامعه ناموفق بود: { $error }
plugins-installed-plugins = → افزونه‌های نصب‌شده
plugins-behavioral-port = بازسازی رفتاری
plugins-source-derived-port = برگرفته از کد منبع
plugins-from-for-original-authors = از { $name } برای { $ecosystem }؛ نویسندگان اصلی: { $authors }.
plugins-original-source-at = کد منبع اصلی در { $revision }
plugins-settings = تنظیمات
plugins-package = بسته
plugins-details-screenshots = جزئیات و تصاویر
plugins-uninstalling = در حال حذف نصب…
plugins-uninstall = حذف نصب…
plugins-plugin-settings-sections = بخش‌های تنظیمات افزونه
plugins-browse = مرور
plugins-installed = نصب‌شده ({ $count })
plugins-experimental-plugin-platform = بستر آزمایشی افزونه
plugins-tine-plugins-are-capability-limited-webassembly-guests-not = افزونه‌های Tine مهمان‌های WebAssembly با توانایی محدودند، نه افزونهٔ Logseq یا Obsidian. نمی‌توانند مستقیم به DOM، Tauri، شبکه، فایل‌ها، فرایندها یا گراف شما دسترسی داشته باشند. هر نسخهٔ افزونه غیرفعال نصب می‌شود و فقط پس از اعتبارسنجی توانایی‌های اعلام‌شده و ورودی‌اش اجرا می‌شود.
plugins-install-a-local-package = نصب بستهٔ محلی
plugins-select-its-manifest-json-and-wasm-file-together = فایل‌های <code>manifest.json</code> و <code>.wasm</code> را با هم انتخاب کنید.
plugins-validating = در حال اعتبارسنجی…
plugins-choose-package = انتخاب بسته…
plugins-community-catalogue = فهرست جامعه
plugins-signed-registry-automated-deterministic-and-no-tools = فهرست امضاشده · ممیزی خودکار قطعی و هوش مصنوعی بدون ابزار · چکیدهٔ تغییرناپذیر نسخه‌ها.
plugins-showing-the-last-verified-offline-copy = آخرین نسخهٔ آفلاین تأییدشده نمایش داده می‌شود.
plugins-installed-plugins-are-held-until-a-signed = افزونه‌های نصب‌شده تا تأیید فهرست امضاشده نگه داشته می‌شوند.
plugins-checking-the-signed-catalogue = در حال بررسی فهرست امضاشده…
plugins-no-verified-catalogue-is-available = فهرست تأییدشده‌ای در دسترس نیست.
plugins-retry = تلاش دوباره
plugins-installed-status = نصب‌شده
plugins-checking = در حال بررسی…
plugins-unavailable-on = در { $platform } در دسترس نیست
plugins-verifying = در حال تأیید…
plugins-install = نصب
plugins-human-written = نوشتهٔ انسان
plugins-ai = AI-{ $developmentMethod }
plugins-checked = بررسی‌شده در { $date }
plugins-hide-safety-report = پنهان کردن گزارش ایمنی
plugins-safety-report = گزارش ایمنی
plugins-verifying-the-signed-report = در حال تأیید گزارش امضاشده…
plugins-the-report-could-not-be-fetched-and = گزارش دریافت و با چکیده تأیید نشد.
plugins-why-human-review-was-required = چرا بازبینی انسانی لازم بود
plugins-an-automated-check-found-behavior-that-tine = بررسی خودکار رفتاری یافت که Tine پیش از انتشار بررسی انسانی آن را لازم می‌داند.
plugins-this-plugin-can-edit-the-focused-block = این افزونه هنگام اجرای فرمانش می‌تواند بلوک فعال را ویرایش کند. Tine هر افزونه‌ای را که در گراف می‌نویسد برای بازبینی انسانی نگه می‌دارد، حتی اگر بررسی‌های خودکار بگذرند.
plugins-the-audit-also-caught-that-an-earlier = ممیزی همچنین نشان داد که پیش‌نویسی قدیمی ممکن بود روی بلوک فعال اشتباهی عمل کند. افزونهٔ منتشرشده به بلوک‌های جدول و تختهٔ پرس‌وجو محدود و دوباره بازبینی شد.
plugins-signed-review-record = سابقهٔ بازبینی امضاشده: { $note }
plugins-severity-describes-possible-impact-not-reviewer-confidence = شدت تأثیر ممکن را نشان می‌دهد، نه اطمینان بازبین. «کم‌خطر» یعنی مشکلی محدود که بعید است به یادداشت‌ها آسیب بزند؛ «اطلاعات» یک مشاهده است، نه آسیبی شناخته‌شده.
plugins-raw-report = گزارش خام
plugins-automated-review-is-evidence-not-a-guarantee = بازبینی خودکار شاهد است، نه تضمین.
plugins-no-plugins-installed = افزونه‌ای نصب نیست.
plugins-open-settings = تنظیمات…
plugins-details = جزئیات…
plugins-capabilities-list = { $kind ->
    [none] توانایی‌ها: هیچ
   *[some] توانایی‌ها: { $capabilities }
    }
plugins-audit-details = منبع <source-code>{ $source }</source-code> · بسته <package-code>{ $package }</package-code> · گزارش <report-code>{ $report }</report-code> · { $count } بخش بازبینی شد
plugins-ui-could-not-cache-verified-plugin-registry = فهرست تأییدشدهٔ افزونه‌ها ذخیره نشد: { $value1 }
plugins-ui-could-not-cache-plugin-safety-report = گزارش ایمنی افزونه ذخیره نشد.
plugins-prose-information = اطلاعات
plugins-prose-human-reviewed-before-publication = بازبینی انسانی پیش از انتشار
plugins-prose-low-risk-automated-pass = گذر خودکار کم‌خطر
plugins-prose-automated-review-passed = بازبینی خودکار گذشت
plugins-prose-risk-finding = { $severity ->
        [low] یافتهٔ کم‌خطر
        [medium] یافتهٔ با خطر متوسط
        [critical] یافتهٔ بحرانی
       *[high] یافتهٔ پرخطر
    }
