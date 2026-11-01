git-settings-label = کامیت تغییرات در git
git-settings-policy-off = خاموش
git-settings-policy-commit = کامیت خودکار
git-settings-policy-sync-on-demand = کامیت و همگام‌سازی به درخواست
git-settings-policy-sync-automatically = کامیت و همگام‌سازی خودکار
git-settings-interval = حداکثر هر
git-settings-interval-aria = دقیقه‌های میان کامیت‌های خودکار
git-settings-minutes = دقیقه
git-settings-hint = وقتی گراف در مخزن git است، Tine تغییرات را پس از یک دقیقه بدون ویرایش کامیت می‌کند، حداکثر یک بار در هر بازه، و پیش و پس از تغییر نام، ادغام، حذف یا بازیابی پشتیبان. فایل‌هایی که git نادیده می‌گیرد کنار می‌مانند. گزینه‌های همگام‌سازی با شاخهٔ بالادست هم pull و push می‌کنند: به درخواست با زدن «همگام‌سازی کنونی»، و خودکار پس از هر کامیت و یک بار در هر بازه. Tine هرگز merge، rebase یا force-push نمی‌کند.
git-settings-not-available = Git هنوز روی این دستگاه در دسترس نیست.
git-settings-not-a-repository = این گراف در مخزن git نیست. می‌توانید اینجا مخزنی بسازید تا تاریخچه‌اش نگه داشته شود.
git-clone-label = کلون گراف از نشانی git
git-init-label = تبدیل این گراف به مخزن git
git-init-remote = نشانی HTTPS مخزن راه دور خالی (اختیاری)
git-init-pending = گراف و نخستین کامیت شما اینجا امن‌اند. برای انتشار آن کامیت در مخزن راه دور خالی، راه‌اندازی را تمام کنید؛ در صورت نیاز پایین وارد شوید.
git-init-finish = پایان راه‌اندازی مخزن
git-setup-url = نشانی HTTPS مخزن
git-setup-https = نشانی HTTPS مخزن را بدون نام کاربری یا توکن وارد کنید.
git-setup-off = کامیت و همگام‌سازی خودکار خاموش می‌مانند. می‌توانید در تنظیمات ← گراف ← Git روشنشان کنید.
git-setup-working = در حال کار…
git-clone-change-location = انتخاب مکان دیگر
git-clone-open = باز کردن گراف کلون‌شده
git-clone-sign-in = ورود و تلاش دوبارهٔ کلون
git-clone-open-failed = کلون در { $path } است. باز کردن را دوباره امتحان کنید یا بعداً از منوی گراف باز کنید.
git-init-failed = تبدیل این گراف به مخزن git تمام نشد: { $error }
git-settings-branch = شاخه: { $branch }
git-settings-unusable = Tine اینجا نمی‌تواند کامیت کند: { $message }
git-settings-last-commit = آخرین کامیت { $commit } در { $time }.
git-settings-last-refused = آخرین تلاش ناموفق بود: { $message }
git-settings-repack-deferred = پاک‌سازی Git در انتظار است؛ کامیت‌ها ادامه دارند. Tine خودکار دوباره تلاش می‌کند.
git-settings-stale-lock = { $path } بیش از ۳۰ دقیقه نگه داشته شده؛ شاید برنامهٔ git از کار افتاده باشد. وقتی هیچ فرمان gitی در حال اجرا نیست حذفش کنید.
git-settings-load-failed = تنظیمات git بارگذاری نشد: { $error }
git-settings-save-failed = تنظیمات git تغییر نکرد: { $error }
git-settings-ignore-added =
    { $which ->
        [tine] Tine عبارت logseq/.tine-* را به { $path } افزود تا فایل‌های مخصوص این دستگاه، مانند زباله‌دانش، بیرون از git بمانند و دستگاه‌هایی که یک مخزن را همگام می‌کنند روی صفحهٔ حذف‌شده متوقف نشوند. رونوشت‌های موجود در git در کامیت جداگانهٔ Tine از git حذف می‌شوند، نه از دیسک. می‌توانید خط را حذف کنید؛ Tine روی این دستگاه دوباره اضافه‌اش نمی‌کند و همچنان آن فایل‌ها را در کامیت‌هایش نمی‌گذارد.
        [both] Tine عبارت‌های .syncthing.* و logseq/.tine-* را به { $path } افزود تا فایل‌های موقت Syncthing و فایل‌های مخصوص این دستگاه، مانند زباله‌دانش، بیرون از git بمانند. رونوشت‌های موجود در git در کامیت جداگانهٔ Tine از git حذف می‌شوند، نه از دیسک. می‌توانید خطوط را حذف کنید؛ Tine روی این دستگاه دوباره اضافه‌شان نمی‌کند. رونوشت‌های تعارض همچنان کامیت می‌شوند، مگر قوانین خودتان آن‌ها را نادیده بگیرند.
       *[syncthing] Tine عبارت .syncthing.* را به { $path } افزود تا فایل‌های موقت Syncthing بیرون از git بمانند. می‌توانید خط را حذف کنید؛ Tine روی این دستگاه دوباره اضافه‌اش نمی‌کند. رونوشت‌های تعارض همچنان کامیت می‌شوند، مگر قوانین خودتان آن‌ها را نادیده بگیرند.
    }
git-settings-ignore-failed =
    { $which ->
        [tine] Tine نتوانست افزودن logseq/.tine-* به .gitignore گراف را تأیید کند. فایل‌های خود Tine همچنان از کامیت‌هایش کنار می‌مانند. فایل را بررسی کنید و اگر می‌خواهید ابزارهای دیگر git هم نادیده‌شان بگیرند، خط را خودتان اضافه کنید.
        [both] Tine نتوانست افزودن .syncthing.* و logseq/.tine-* به .gitignore گراف را تأیید کند. فایل‌های موقت و فایل‌های خود Tine همچنان از کامیت‌هایش کنار می‌مانند. فایل را بررسی کنید و اگر می‌خواهید ابزارهای دیگر git هم نادیده‌شان بگیرند، خطوط را خودتان اضافه کنید.
       *[syncthing] Tine نتوانست افزودن .syncthing.* به .gitignore گراف را تأیید کند. فایل‌های موقت همچنان از کامیت‌هایش کنار می‌مانند. فایل را بررسی کنید و اگر می‌خواهید ابزارهای دیگر git هم نادیده‌شان بگیرند، خط را خودتان اضافه کنید.
    }
git-settings-remote = مخزن راه دور: { $name } ({ $url })
git-settings-sync-unusable = Tine اینجا نمی‌تواند همگام‌سازی کند: { $message }
git-settings-sync-now = همگام‌سازی کنونی
git-settings-sync-now-failed = همگام‌سازی شروع نشد: { $error }
git-settings-signed-in = به‌عنوان { $username } در { $origin } وارد شده‌اید.
git-settings-not-signed-in = در { $origin } وارد نشده‌اید.
git-settings-sign-in = ورود
git-settings-sign-out = خروج
git-settings-sign-in-hint = از توکن دسترسی شخصی با اجازهٔ خواندن و نوشتن مخزن استفاده کنید: در GitHub در Settings → Developer settings → Personal access tokens، در GitLab در Preferences → Access tokens، در Codeberg در Settings → Applications. Tine آن را در فایلی خصوصی روی این دستگاه نگه می‌دارد، هرگز در گراف.
git-settings-username = نام کاربری
git-settings-token = توکن
git-settings-sign-in-save = ذخیرهٔ ورود
git-settings-sign-in-cancel = انصراف
git-settings-sign-in-failed = ورود انجام نشد: { $error }
git-settings-sign-out-failed = خروج انجام نشد: { $error }
git-sync-running = در حال همگام‌سازی…
git-sync-last-up-to-date = آخرین همگام‌سازی در { $time }: از قبل به‌روز بود.
git-sync-last-pulled = آخرین همگام‌سازی در { $time }: تغییرات جدید دریافت شد.
git-sync-last-pushed = آخرین همگام‌سازی در { $time }: کامیت‌های این دستگاه ارسال شد.
git-sync-last-failed = آخرین همگام‌سازی در { $time } تمام نشد.
git-sync-last-failed-since = آخرین همگام‌سازی در { $time } تمام نشد؛ آخرین موفق در { $ok } بود.
git-sync-diverged = این دستگاه و مخزن راه دور هر دو کامیت جدید دارند، پس Tine همگام‌سازی را متوقف کرد و چیزی را تغییر نداد. آن‌ها را در git ادغام کنید: فرمان زیر را در ترمینال اجرا کنید، تعارض‌ها را حل و کامیت کنید، سپس «همگام‌سازی کنونی» را بزنید. (ادغام درون Tine بعداً می‌آید.)
git-sync-auth-required = مخزن راه دور ورود می‌خواهد یا توکن Tine را نپذیرفت. با توکن جدید دوباره وارد شوید.
git-sync-network = Tine به مخزن راه دور نرسید ({ $message }). بعداً دوباره تلاش می‌کند.
git-sync-held-unsaved = Tine تا ذخیرهٔ ویرایش شما در { $path } برای به‌روزرسانی آن صبر می‌کند. در همگام‌سازی بعدی دوباره تلاش می‌کند.
git-sync-held-changed = { $path } هنگام همگام‌سازی بیرون از Tine تغییر کرد، پس Tine چیزی را به‌روز نکرد. در همگام‌سازی بعدی Tine آن تغییر را کامیت می‌کند؛ چون مخزن راه دور هم کامیت جدید دارد، بعد باید آن‌ها را در git ادغام کنید.
git-sync-held-changed-unnamed = فایلی هنگام همگام‌سازی بیرون از Tine تغییر کرد، پس Tine چیزی را به‌روز نکرد. در همگام‌سازی بعدی دوباره تلاش می‌کند.
git-sync-busy = فرمان git دیگری از مخزن استفاده می‌کند. Tine در همگام‌سازی بعدی دوباره تلاش می‌کند.
git-sync-incomplete = Tine نتوانست به‌روزرسانی گراف را تمام کند ({ $message }). چیزی از بین نرفته است: در همگام‌سازی بعدی تمام می‌شود.
git-sync-pull-refused = Tine نمی‌تواند { $path } را به‌روز کند: { $message }. چیزی تغییر نکرد. در git اصلاحش کنید و سپس «همگام‌سازی کنونی» را بزنید.
git-sync-no-upstream = این شاخه شاخهٔ بالادستی برای همگام‌سازی ندارد. در git تعیینش کنید و سپس «همگام‌سازی کنونی» را بزنید.
git-sync-refused = Tine نمی‌تواند این مخزن را همگام‌سازی کند: { $message }
git-sync-pull-stopped = Tine به‌روزرسانی گراف را در { $path } متوقف کرد: { $message }. برخی فایل‌ها از قبل به‌روز شده‌اند. در git اصلاحش کنید و سپس «همگام‌سازی کنونی» را بزنید.
git-sync-other = همگام‌سازی متوقف شد: { $message }. در git اصلاحش کنید و سپس «همگام‌سازی کنونی» را بزنید.
git-sync-other-path = همگام‌سازی در { $path } متوقف شد: { $message }. شاید برخی فایل‌ها به‌روز شده باشند. در git اصلاحش کنید و سپس «همگام‌سازی کنونی» را بزنید.
git-sync-fix-copy-command = کپی فرمان git
git-sync-fix-open-folder = باز کردن پوشه
git-sync-fix-sign-in = ورود دوباره
git-sync-copy-failed = فرمان کپی نشد.
git-sync-open-folder-failed = پوشه باز نشد: { $error }
git-sync-notice-title = همگام‌سازی Git نیاز به توجه دارد
git-sync-notice-settings = تنظیمات Git
git-sync-fix-line-endings = خاموش کردن تبدیل پایان خط برای این مخزن
git-sync-fix-line-endings-guide = پایان خط: مراحل راهنما
git-sync-line-endings-safe = تبدیل خودکار پایان خط در Git جلوی همگام‌سازی را می‌گیرد. همهٔ فایل‌های رهگیری‌شده با بایت‌های کامیت‌شده‌شان یکسان‌اند، پس Tine می‌تواند تبدیل را اینجا بدون تغییر هیچ فایلی خاموش کند.
git-sync-line-endings-differ = { $count } فایل رهگیری‌شده بایت‌های متفاوتی دارند یا خوانده نمی‌شوند، مثلاً { $path }. علتش می‌تواند پایان خط Windows یا ویرایش‌های کامیت‌نشده باشد. ابتدا ویرایش‌ها را کامیت کنید و سپس طبق مراحل راهنما فایل‌ها را بدون تبدیل checkout کنید.
git-sync-line-endings-blocked = خاموش کردن تبدیل خودکار پایان خط هم همگام‌سازی را باز نمی‌کند: { $reason }. مراحل راهنما را دنبال و دوباره امتحان کنید.
git-sync-line-endings-failed = تبدیل پایان خط خاموش نشد: { $error }
git-stale-lock-notice-title = کامیت‌های Git در انتظارند
git-sync-stale-lock = فایل قفل { $path } از { $time } تغییر نکرده و خود Tine از git استفاده نمی‌کند. به احتمال زیاد وقتی Tine یا برنامهٔ git دیگری وسط کامیت متوقف شده جا مانده است. تا وقتی هست، Tine نمی‌تواند کامیت کند. اگر الان هیچ برنامهٔ gitی در این مخزن کار نمی‌کند، حذفش کنید.
git-sync-fix-remove-stale-lock = حذف قفل جامانده
git-sync-stale-lock-failed = قفل حذف نشد: { $error }
