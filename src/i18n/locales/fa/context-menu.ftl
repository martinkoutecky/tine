context-menu-include-parent-block = شامل بلوک والد
context-menu-file-open-failed =
    { $action ->
        [show] فایل صفحه نمایش داده نشد. ({ $error })
       *[open] فایل صفحه باز نشد. ({ $error })
    }
context-menu-file-not-saved = این صفحه هنوز فایلی روی دیسک ندارد. چیزی بنویسید و بگذارید Tine ابتدا ذخیره کند.
context-menu-child-view = { $active ->
        [yes] { "✓ " }
       *[no] { "" }
    }{ $view ->
        [grid] شبکه
        [table] جدول
       *[outline] طرح کلی
    }
context-menu-delete-page-label =
    { $kind ->
        [journal] حذف روزنوشت
       *[page] حذف صفحه
    }
context-menu-ui-couldn-t-copy-clipboard-write-failed = کپی نشد: نوشتن در کلیپ‌بورد ناموفق بود.
context-menu-ui-page-actions = عملیات صفحه
context-menu-ui-show-children-as = نمایش فرزندان به‌صورت ←
context-menu-ui-automatic-heading = عنوان خودکار
context-menu-ui-auto = خودکار
context-menu-ui-heading = عنوان { $h }
context-menu-ui-remove-heading = حذف عنوان
context-menu-ui-no-background = بدون پس‌زمینه
context-menu-ui-add-child-bullet = افزودن گلولهٔ فرزند
context-menu-ui-zoom-into-cell = تمرکز روی خانه
context-menu-ui-delete-row = حذف ردیف
context-menu-ui-delete-column = حذف ستون
context-menu-ui-open-as-full-page = باز کردن به‌صورت صفحهٔ کامل
context-menu-ui-add-formula = افزودن فرمول…
context-menu-ui-edit-filter = ویرایش پالایه…
context-menu-ui-group-by = گروه‌بندی بر اساس ←
context-menu-ui-no-grouping = { $choice1 ->
        [yes] { "✓ " }
       *[no] { "" }
    }بدون گروه‌بندی
context-menu-ui-no-structural-actions = عملیات ساختاری نیست
context-menu-ui-convert-to-pipe-table = تبدیل به جدول Markdown
context-menu-ui-hierarchify-into-columns = سلسله‌مراتبی کردن به ستون‌ها
context-menu-ui-hierarchify-by = سلسله‌مراتبی بر اساس ←
context-menu-ui-flatten = تخت کردن
context-menu-ui-open-in-sidebar = باز کردن در نوار کناری
context-menu-ui-go-to-block = رفتن به بلوک
context-menu-ui-copy-link = کپی پیوند
context-menu-ui-copy-as = کپی به‌صورت
context-menu-ui-copy-block-ref = کپی ارجاع بلوک
context-menu-ui-copied-block-ref = ارجاع بلوک کپی شد
context-menu-ui-copy-block-embed = کپی جاسازی بلوک
context-menu-ui-copied-block-embed = جاسازی بلوک کپی شد
context-menu-ui-a-template-named-already-exists = قالبی به نام «{ $title }» از قبل هست.
context-menu-ui-template-created = قالب «{ $title }» ساخته شد.
context-menu-ui-make-a-template = ساختن قالب…
context-menu-ui-template-name = نام قالب
context-menu-ui-create-template = ساختن قالب
context-menu-ui-tip-in-the-template-expand-when-it-s-inserted = نکته: در قالب، <code><% today %></code>، <code><% current page %></code> و <code><% date: +3d %></code> هنگام درج (یا با <code>/Template var</code>) جایگزین می‌شوند.
context-menu-ui-this-page-target-changed-reopen-the-page-actions-menu = مقصد این صفحه تغییر کرد؛ منوی عملیات صفحه را دوباره باز کنید.
context-menu-ui-couldn-t-save-its-on-disk-file-was-not = «{ $name }» ذخیره نشد؛ فایلش روی دیسک باز نشد.
context-menu-ui-has-an-unresolved-save-conflict-this-is-the-file = «{ $name }» تعارض ذخیرهٔ حل‌نشده دارد؛ این فایل همان است که روی دیسک هست. تغییرات ذخیره‌نشدهٔ شما تا حل آن در Tine می‌مانند.
context-menu-ui-delete-the-file-moves-to-the-graph-s-tine =
    { $unsaved ->
        [discard] «{ $name }» حذف شود؟ فایل به پوشهٔ .tine-trash گراف می‌رود. تغییرات ذخیره‌نشده هم دور انداخته می‌شوند.
       *[none] «{ $name }» حذف شود؟ فایل به پوشهٔ .tine-trash گراف می‌رود.
    }
context-menu-ui-deleted = «{ $name }» حذف شد
context-menu-ui-open = باز کردن
context-menu-ui-open-in-new-tab = باز کردن در زبانهٔ جدید
context-menu-ui-open-in-new-window = باز کردن در پنجرهٔ جدید
context-menu-ui-remove-from-favorites = حذف از برگزیده‌ها
context-menu-ui-add-to-favorites = افزودن به برگزیده‌ها
context-menu-ui-copy-page-ref = کپی ارجاع صفحه
context-menu-ui-copied-page-ref = ارجاع صفحه کپی شد
context-menu-ui-copy-page-embed = کپی جاسازی صفحه
context-menu-ui-copied-page-embed = جاسازی صفحه کپی شد
context-menu-ui-copy-export-as = کپی / خروجی به‌صورت…
context-menu-ui-share = اشتراک‌گذاری…
context-menu-ui-copy-page-as-markdown = کپی صفحه به‌صورت Markdown
context-menu-ui-copied-page-as-markdown = صفحه به‌صورت Markdown کپی شد
context-menu-ui-couldn-t-copy-page-as-markdown = صفحه به‌صورت Markdown کپی نشد.
context-menu-ui-export-to-pdf = خروجی PDF…
context-menu-ui-show-in-folder = نمایش در پوشه
context-menu-ui-open-with-default-app = باز کردن با برنامهٔ پیش‌فرض
context-menu-ui-find-in-page = یافتن در صفحه
context-menu-ui-page-properties = ویژگی‌های صفحه…
context-menu-ui-carry-unfinished-tasks-today = انتقال وظیفه‌های ناتمام ← امروز
context-menu-rename-success =
    { $outcome ->
        [merged] در «{ $next }» ادغام شد
       *[renamed] نام به «{ $next }» تغییر کرد
    }
context-menu-ui-rename-failed = تغییر نام ناموفق بود: { $value1 }
context-menu-ui-rename-page = تغییر نام صفحه…
context-menu-ui-copied = بلوک کپی شد
context-menu-ui-zoom-into-block = تمرکز روی بلوک
context-menu-ui-copy = کپی بلوک
context-menu-ui-properties = ویژگی‌ها…
context-menu-ui-insert-block-above = درج بلوک در بالا
context-menu-ui-copy-block-refs = کپی ارجاع‌های بلوک
context-menu-ui-copy-block-embeds = کپی جاسازی‌های بلوک
context-menu-ui-convert-to-grid = تبدیل به شبکه
context-menu-ui-cut = برش بلوک
context-menu-ui-couldn-t-cut-clipboard-write-failed = بلوک بریده نشد: نوشتن در کلیپ‌بورد ناموفق بود.
context-menu-ui-remove-numbered-list = حذف فهرست شماره‌دار
context-menu-ui-numbered-list = فهرست شماره‌دار
context-menu-ui-collapse-all = جمع کردن همه
context-menu-ui-expand-all = باز کردن همه
context-menu-ui-used-for-new-journals = ✓ برای روزنوشت‌های جدید به کار می‌رود
context-menu-ui-use-for-new-journals = استفاده برای روزنوشت‌های جدید
context-menu-ui-delete = حذف بلوک
