app-quit-without-saving-body = { app-close-explanation }{ "\u000A" }{ "\u000A" }{ app-close-inventory }{ "\u000A" }{ "\u000A" }برگردید تا بررسی کنید، دوباره ذخیره کنید یا پیش‌نویس‌ها را کپی کنید. یا آن‌ها را دور بیندازید و خارج شوید.
app-quit-without-saving-title = { $count ->
    [0] خروج بدون ذخیره؟
   *[other] خروج بدون ذخیرهٔ { NUMBER($count, useGrouping: 0) } صفحه؟
    }
app-couldn-t-save-pending-pdf-changes-the-graph = تغییرات در انتظار PDF ذخیره نشد. گراف باز می‌ماند.
app-still-saving-your-changes-closing-in = هنوز در حال ذخیرهٔ تغییرات؛ به‌زودی بسته می‌شود.
app-couldn-t-confirm-closing-the-window-your-unsaved = بستن پنجره تأیید نشد. تغییرات ذخیره‌نشده هنوز بازند.
app-tine-couldn-t-close-the-android-activity-tap = Tine نتوانست activity اندروید را ببندد. برای تلاش دوباره «بازگشت» را بزنید.
app-close-tab = بستن زبانه
app-pane-select = انتخاب قاب
app-move-onto-seams-edges = <kbd>←</kbd><kbd>→</kbd><kbd>↑</kbd><kbd>↓</kbd> جابه‌جایی (روی مرزها و لبه‌ها) · <kbd>Enter</kbd> ورود به قاب · <kbd>Del</kbd> بستن قاب
app-ctrl-k-open-a-page-in-this-pane = <kbd>Ctrl+K</kbd> باز کردن صفحه در این قاب · <kbd>Esc</kbd> خروج
app-captured-to = در «{ $title }» ثبت شد
app-captured-to-today-s-journal = در روزنوشت امروز ثبت شد
app-capture-couldn-t-be-saved-its-text = یادداشت سریع ذخیره نشد؛ متنش در پنجرهٔ یادداشت نگه داشته شده است
app-tine-was-renamed-under-the-hood-so = نام داخلی Tine عوض شد، پس تنظیمات و پشتیبان‌ها را منتقل کردیم. شاید لازم باشد چند ترجیح برنامه (مثل میان‌برهای صفحه‌کلید) را دوباره تنظیم کنید؛ پوزش می‌خواهیم!
app-tine-could-not-write-its-usual-application-data = Tine نتوانست در پوشهٔ معمول دادهٔ برنامه بنویسد، پس این نشست تنظیمات و پشتیبان‌ها را در { $fallbackDirectory } نگه می‌دارد. اصلاح مجوزهای آن پوشه مکان عادی را برمی‌گرداند.
app-drafts-on-volatile-storage = Tine دادهٔ برنامه را در حافظهٔ موقت نگه می‌دارد. پیش‌نویس‌های ذخیره‌نشده ممکن است پس از راه‌اندازی دوباره از بین بروند. پیش از راه‌اندازی دوبارهٔ دستگاه، ویرایش‌ها را در گراف ذخیره کنید.
app-quick-capture-could-not-connect-to-this = یادداشت سریع نتوانست به پنجرهٔ این گراف وصل شود.
app-the-block-renderer-failed-to-load = نمایشگر بلوک بارگذاری نشد و متن بدون قالب‌بندی نشان داده می‌شود. لطفاً Tine را دوباره بارگذاری کنید؛ اگر ادامه داشت، گزارش دهید.
app-finishing-graph-operation = در حال پایان دادن به عملیات گراف…
app-close = بستن
app-close-navigation-sidebar = بستن نوار کناری پیمایش
app-journals = روزنوشت‌ها
app-toggle-theme-t-t = تغییر پوسته (t t)
app-toggle-right-sidebar-t-r = نمایش یا پنهان کردن نوار کناری راست (t r)
app-settings-t-s = تنظیمات (t s)
app-refreshing-changes-from-disk = در حال تازه‌سازی تغییرات از دیسک…
app-exit-focus-esc = خروج از تمرکز (Esc)
app-close-explanation = { $reason ->
    [still-saving] Tine هنوز در حال نوشتن تغییرات شماست و بیش از حد انتظار طول کشیده است؛ درایو کند یا شبکه‌ای می‌تواند علت باشد.
   *[other] Tine تغییراتی دارد که ذخیره نشدند (تعارض یا ذخیرهٔ گیرکرده).
    }
app-close-inventory = { $inventory ->
    [empty] پیوست‌ها یا کار ذخیره‌سازی در انتظار است؛ پیش‌نویس صفحه‌ای شناسایی نشد.
   *[other] { $inventory }
    }
app-pane-split-instruction = { $kind ->
    [edge] <kbd>Enter</kbd> <span>تقسیم <em-hint>کل پنجره</em-hint></span> · <em-hint>نام صفحه را بنویسید</em-hint> (یا <kbd>Ctrl+K</kbd>) تا در قاب جدید باز شود
    [pane-edge] <kbd>Enter</kbd> <span>تقسیم <em-hint>این قاب</em-hint></span> · <em-hint>نام صفحه را بنویسید</em-hint> (یا <kbd>Ctrl+K</kbd>) تا در قاب جدید باز شود
   *[seam] <kbd>Enter</kbd> <span>تقسیم در اینجا (قاب را تکرار می‌کند)</span> · <em-hint>نام صفحه را بنویسید</em-hint> (یا <kbd>Ctrl+K</kbd>) تا در قاب جدید باز شود
    }
app-pane-split-navigation = { $kind ->
    [pane-edge] <span>برای پهن‌تر کردن تقسیم دوباره به بیرون بروید · </span><kbd>←</kbd><kbd>→</kbd><kbd>↑</kbd><kbd>↓</kbd> جابه‌جایی · <kbd>Esc</kbd> خروج
   *[other] <kbd>←</kbd><kbd>→</kbd><kbd>↑</kbd><kbd>↓</kbd> جابه‌جایی · <kbd>Esc</kbd> خروج
    }
app-prose-navigation-sidebar = نوار کناری پیمایش
app-prose-unexpected-failure = خطای غیرمنتظره.
capture-snapshot-saved = یادداشت قبلی ذخیره شد؛ متن ارسال‌شده در جاهای بدون تغییر حذف و ویرایش‌های جدید نگه داشته شد
capture-title-needs-text = برای ثبت این عنوان متن اضافه کنید؛ عنوان نگه داشته شد
capture-retry-unconfirmed = ذخیره تأیید نشده است و متن نگه داشته شد. برای پایان همین یادداشت، ثبت را دوباره امتحان کنید.
capture-previous-graph = شاید یادداشت در گراف قبلی ثبت شده باشد؛ ذخیره تأیید نشده و متن نگه داشته شد. پیش از یادداشت دوباره آن گراف را بررسی کنید.
capture-start-new = شروع یادداشت جدید
capture-new-request = متن برای یادداشت جدید نگه داشته شد. پیش از ثبت دوباره، هرچه را در گراف قبلی ذخیره شده حذف کنید.
