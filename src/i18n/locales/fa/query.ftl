query-text-vocabulary-guide = فیلدها و ویژگی‌های این گراف در انتخابگر بالا هستند؛ زبان پرس‌وجو در راهنما زیر <em>Find and revisit → Query text (TQL)</em> است.
query-registry-failed = ویژگی‌های این گراف خوانده نشد. { $detail }
query-anchor-unchanged = لنگر تغییر نکرد: { $detail }
query-grouped-by = گروه‌بندی با { $field }
query-skipped-non-numeric = { " " }({ NUMBER($count, useGrouping: 0) } مورد غیرعددی رد شد)
query-skipped = { " " }({ NUMBER($count, useGrouping: 0) } مورد رد شد)
query-og-bare-task = <code>{ $syntax }</code> بدون نشانگر شرطی نمی‌افزاید، مانند Logseq.{ " " }
query-og-bare-priority = <code>{ $syntax }</code> بدون سطح شرطی نمی‌افزاید، مانند Logseq.{ " " }
query-preview-load-failed = نتایج بارگذاری نشد: { $detail }
query-preview-count =
    { $kind ->
        [page] { NUMBER($count, useGrouping: 0) } صفحه
       *[other] { NUMBER($count, useGrouping: 0) } نتیجه
    }
query-preview-limit = { " " }(نمایش { NUMBER($count, useGrouping: 0) } مورد نخست)
query-preview-families = { NUMBER($pages, useGrouping: 0) } صفحه · { NUMBER($blocks, useGrouping: 0) } بلوک
query-preview-family-limit = { " " }(نمایش { NUMBER($count, useGrouping: 0) } مورد نخست از هر کدام)
query-preview-pages-note = صفحه‌ها: { $detail }
query-preview-blocks-note = بلوک‌ها: { $detail }
query-workspace-result-count = { NUMBER($count, useGrouping: 0) } نتیجه
query-workspace-search-failed = جستجو ناموفق بود: { $detail }
query-workspace-explanation =
    { $hasWithout ->
        [1] { $conjunct }: به‌تنهایی { NUMBER($alone, useGrouping: 0) }، بدون آن { NUMBER($without, useGrouping: 0) }
       *[other] { $conjunct }: به‌تنهایی { NUMBER($alone, useGrouping: 0) }
    }
query-selected-count = { NUMBER($count, useGrouping: 0) } انتخاب‌شده
query-anchor-label =
    { $anchor ->
        [page] صفحه‌ها
        [both] صفحه‌ها و بلوک‌ها
       *[other] بلوک‌ها
    }
query-anchor-switch = تغییر به <strong>{ query-anchor-label }</strong>: { NUMBER($count, useGrouping: 0) } از { NUMBER($total, useGrouping: 0) } شرط شما روی { query-anchor-label } اعمال نمی‌شود{ $hasNames ->
        [1] { " " }(<code>{ $names }</code>)
       *[other] { "" }
    }.
query-display-change = تغییر
query-sort-direction = مرتب‌سازی { $field } { $direction ->
        [asc] نزولی
       *[other] صعودی
    }
query-display-add-sort = + مرتب‌سازی
query-display-add-column = + ستون
query-display-add-property = + ویژگی
query-display-whole-result = کل نتیجه
query-search-positive-term = دست‌کم یک عبارت جستجوی مثبت بیفزایید.
query-unsupported-clause = این بند پرس‌وجو هنوز پشتیبانی نمی‌شود: { $clause }
query-fields-regex-warning = عبارت باقاعده هنوز با فیلدهای سادهٔ دیگر ترکیب نمی‌شود.
query-options-map-warning = نگاشت گزینه‌ها (title، collapsed) با کنترل‌های عنوان و نمایش ویرایش می‌شود، نه اینجا.
query-save-superseded = این فضای کار هنگام ذخیره تغییر کرد، پس چیزی نوشته نشد. دوباره ذخیره کنید.
query-save-enter-title = پیش از ذخیره عنوان صفحه را وارد کنید.
query-save-enter-query = پیش از ذخیره جستجو یا پرس‌وجو را وارد کنید.
query-save-graph-changed = گراف پیش از ذخیرهٔ این فضای کار تغییر کرد.
query-save-validation-superseded = اعتبارسنجی جستجو با درخواست جدیدتری جایگزین شد. دوباره ذخیره کنید.
query-save-included-term = پیش از ذخیره جستجویی با دست‌کم یک عبارت شامل‌شده وارد کنید.
query-save-validation-detail = این جستجو اعتبارسنجی نشد: { $detail }
query-save-validation-failed = این جستجو اعتبارسنجی نشد.
query-save-title-exists = صفحه‌ای به نام «{ $name }» از قبل هست. عنوان دیگری انتخاب کنید.
query-save-title-alias = «{ $name }» نام مستعار صفحه‌ای موجود است. عنوان دیگری انتخاب کنید.
query-save-local-state-changed = «{ $name }» ذخیره نشد، چون وضعیت محلی صفحه‌اش تغییر کرد. دوباره امتحان کنید.
query-save-conflict = «{ $name }» پیش از ذخیرهٔ این فضای کار ساخته یا تغییر داده شد. بازنویسی نشده است.
query-save-failed-detail = «{ $name }» ذخیره نشد: { $detail }
query-save-failed = «{ $name }» ذخیره نشد.
query-fields-quotation-warning = گیومه درون این فیلدها پشتیبانی نمی‌شود.
query-fields-include-warning = دست‌کم یک کلمه یا عبارت دقیق برای شامل شدن بیفزایید.
query-dsl-empty-warning = DSL پرس‌وجو نمی‌تواند خالی باشد.
query-save-earlier-search = «{ $name }» از جستجوی قبلی ذخیره شد، پس این فضای کار همان‌طور ماند.
query-workspace-view-search = جستجو
query-workspace-view-list = فهرست
query-workspace-view-table = جدول
query-workspace-view-board = تخته
query-display-view-search = جستجو
query-display-view-list = فهرست
query-display-view-table = جدول
query-display-view-board = تخته
query-display-aggregate-count = شمارش
query-display-aggregate-sum = جمع
query-display-aggregate-avg = میانگین
query-export-dialog-export-query-results = خروجی گرفتن از نتایج پرس‌وجو
query-export-dialog-name = نام
query-export-dialog-resolving-pages = در حال یافتن صفحه‌ها…
query-export-dialog-journal = { " " }· روزنوشت
query-export-dialog-all-blocks-on-these-pages-will-be-exported = همهٔ بلوک‌های این صفحه‌ها خروجی گرفته می‌شوند، حتی بلوک‌هایی که مطابق نبودند.
query-export-dialog-destination = مقصد
query-export-dialog-replace-it-the-previous-export-is-kept-in = { " " }جایگزین شود (خروجی قبلی در بازیابی نگه داشته می‌شود)
query-export-dialog-adjust-limit-in-settings = تنظیم محدودیت در تنظیمات…
query-export-dialog-cancel = انصراف
query-export-dialog-exporting = در حال خروجی گرفتن…
query-export-dialog-export = خروجی
query-export-limit-setting-query-export-size-limit = محدودیت اندازهٔ خروجی پرس‌وجو
query-export-limit-setting-query-export-size-limit-mi-b = محدودیت اندازهٔ خروجی پرس‌وجو (MiB)
query-export-limit-setting-default-1-gi-b = پیش‌فرض: 1 GiB
query-export-limit-setting-mi-b = { " " }MiB
query-export-limit-setting-reset = بازنشانی
query-export-limit-setting-default-1-gi-b-copied-assets-must-fit = پیش‌فرض: 1 GiB. پیوست‌های کپی‌شده باید در این محدودیت جا شوند؛ خروجی بزرگ‌تر متوقف می‌شود. روی این دستگاه ذخیره می‌شود.
query-legacy-table-content = محتوا
query-legacy-table-page = صفحه
query-listbox-type-to-filter = برای پالایش تایپ کنید
query-page-results-page = صفحه
query-page-results-journal = روزنوشت
query-page-results-page-label = صفحه
query-page-results-page-results = نتایج صفحه
query-page-results-page-results-grouped = نتایج صفحهٔ گروه‌بندی‌شده
query-page-results-no-value = بدون مقدار
query-result-sections-pages = صفحه‌ها
query-result-sections-blocks = بلوک‌ها
query-result-sections-searching = در حال جستجو…
query-result-sections-no-matching-pages = صفحهٔ مطابقی نیست.
query-result-sections-no-matching-blocks = بلوک مطابقی نیست.
query-vocabulary-picker-condition-field = فیلد شرط
query-vocabulary-picker-search-fields-and-properties = جستجوی فیلدها و ویژگی‌ها
query-vocabulary-picker-reading-this-graph-s-properties = در حال خواندن ویژگی‌های این گراف…
query-vocabulary-picker-try-again = تلاش دوباره
query-vocabulary-picker-declared = { $declared } (اعلام‌شده)
sheet-aggregate-footer-hide-aggregate-footer = پنهان کردن پانویس تجمیع
sheet-aggregate-footer-show-aggregate-footer = نمایش پانویس تجمیع
sheet-aggregate-footer-add-aggregate = افزودن تجمیع
sheet-aggregate-footer-change-aggregate = تغییر تجمیع ({ $active_label })
sheet-grid-show-previous-rows = نمایش ردیف‌های قبلی
sheet-grid-show-previous-columns = نمایش ستون‌های قبلی
sheet-grid-add-column = افزودن ستون
sheet-grid-add-row = افزودن ردیف
sheet-grid-cell-menu = منوی خانه
crossing-notice-warning =
    { $has_feature ->
        [1] این پرس‌وجو اکنون از قابلیت‌هایی از Tine استفاده می‌کند که Logseq نمی‌خواند (<code>{ $feature }</code>). Logseq بلوک را متن ساده نشان می‌دهد.
       *[other] این پرس‌وجو اکنون از قابلیت‌هایی از Tine استفاده می‌کند که Logseq نمی‌خواند. Logseq بلوک را متن ساده نشان می‌دهد.
    }
crossing-notice-changed-block = اکنون متن بلوک این است: <code>{ $text }</code>
query-export-dialog-summary = { $results } نتیجه در { $pages } صفحهٔ کامل
query-export-dialog-existing = خروجی‌ای به نام <code>{ $folder }</code> از قبل هست.
query-export-dialog-separate = ساختن خروجی جداگانه با نام <code>{ $folder }</code>
query-export-dialog-path = مقصد: <code>{ $path }</code>. این صفحه‌ها به‌صورت سایت ایستا و برنامهٔ فقط‌خواندنی خروجی گرفته می‌شوند. بارگذاری نمی‌شود. ویرایش‌های ذخیره‌نشده شامل نمی‌شوند.
query-page-results-alias = مطابق با نام مستعار { $alias }
query-result-sections-more-matches =
    { $kind ->
        [page] صفحه‌های مطابق بیش از موارد نمایش‌داده‌شده است.
       *[other] بلوک‌های مطابق بیش از موارد نمایش‌داده‌شده است.
    }
query-vocabulary-picker-registry-failed = ویژگی‌های این گراف خوانده نشد. { $detail }
property-type-mismatches = { $count } بلوک یا صفحه مطابقت ندارد
sheet-grid-more-rows =
    { $load ->
        [1] بارگذاری ردیف‌های بیشتر ({ $start }-{ $end } از { $total })
       *[other] نمایش ردیف‌های بعدی ({ $start }-{ $end } از { $total })
    }
sheet-grid-more-columns =
    { $load ->
        [1] بارگذاری ستون‌های بیشتر ({ $start }-{ $end } از { $total })
       *[other] نمایش ستون‌های بعدی ({ $start }-{ $end } از { $total })
    }
query-vocabulary-picker-use-property =
    { $unit ->
        [pages] استفاده از «{ $key }» به‌عنوان ویژگی صفحه
       *[other] استفاده از «{ $key }» به‌عنوان ویژگی بلوک
    }
query-date-window-refusal = Logseq در این شکل پرس‌وجو بازه‌های تاریخ زمان‌بندی/مهلت را نادیده می‌گیرد، پس این پرس‌وجو به این شکل اینجا اجرا نمی‌شود. برای حفظ معنای بازهٔ تاریخ آن را به پرس‌وجوی Tine تبدیل کنید.
query-convert = تبدیل
query-undo-conversion = واگرد تبدیل
query-query-sheet-parts-click-to-edit-this-query = برای ویرایش این پرس‌وجو کلیک کنید
query-query-sheet-parts-edit-query-filter = ویرایش پالایهٔ پرس‌وجو
query-query-sheet-parts-task-marker = نشانگر وظیفه
query-query-sheet-parts-drag-to-reorder-or-into-another-group-or-use-the = برای تغییر ترتیب یا بردن به گروه دیگر بکشید، یا از کلیدهای پیکان بالا و پایین استفاده کنید
query-query-sheet-parts-the-group-above-is-disabled-so-this-does-not-run = گروه بالا غیرفعال است، پس این اجرا نمی‌شود
query-query-sheet-parts-disabled-kept-in-the-query-not-run = غیرفعال: در پرس‌وجو می‌ماند، اجرا نمی‌شود
query-query-sheet-parts-running-press-to-disable-without-removing-it = در حال اجرا: برای غیرفعال کردن بدون حذف فشار دهید
query-query-sheet-parts-move-up = انتقال به بالا
query-query-sheet-parts-move-down = انتقال به پایین
query-query-sheet-parts-group-with-row-above-all-of = گروه با ردیف بالا: همهٔ
query-query-sheet-parts-group-with-row-above-any-of = گروه با ردیف بالا: هر یک از
query-query-sheet-parts-group-with-row-above-none-of = گروه با ردیف بالا: هیچ‌یک از
query-query-sheet-parts-apply = اعمال
query-query-sheet-parts-start-today-30d-2026-06-01-or-a-page = شروع: today، -30d، 2026-06-01 یا یک صفحه
query-query-sheet-parts-end-today-7d-2026-06-30-or-a-page = پایان: today، +7d، 2026-06-30 یا یک صفحه
query-query-sheet-parts-page-or-tag-name = نام صفحه یا برچسب
query-query-sheet-parts-any-status = هر وضعیت
query-query-sheet-parts-page-name = نام صفحه
query-query-sheet-parts-namespace-parent-page = فضای نام (صفحهٔ والد)
query-query-sheet-parts-text-to-search-for = متن مورد جستجو
query-query-sheet-parts-search-words-and-operators = کلمات و عملگرهای جستجو
query-query-sheet-parts-tag-one = برچسب (یکی)
query-query-sheet-parts-blocks = بلوک‌ها
query-query-sheet-parts-individual-bullets-anywhere-in-the-graph = گلوله‌های تکی، هر جای گراف
query-query-sheet-parts-pages = صفحه‌ها
query-query-sheet-parts-whole-pages = صفحه‌های کامل
query-query-sheet-parts-pages-and-blocks = صفحه‌ها و بلوک‌ها
query-query-sheet-parts-pages-above-matching-blocks-below = صفحه‌ها بالا، بلوک‌های مطابق پایین
query-query-sheet-parts-find = یافتن
query-query-sheet-parts-what-this-query-selects = این پرس‌وجو چه چیزی را انتخاب می‌کند
query-query-sheet-parts-where = جایی که …
query-query-sheet-parts-switching-what-this-query-selects = تغییر آنچه این پرس‌وجو انتخاب می‌کند
query-query-sheet-parts-remove-them = حذفشان کن
query-query-sheet-parts-keep-anyway = با این حال نگه دار
query-query-sheet-parts-cancel = انصراف
query-query-sheet-parts-selected-conditions = شرط‌های انتخاب‌شده
query-query-sheet-parts-select-two-or-more-conditions-to-group-them = برای گروه‌بندی دو شرط یا بیشتر انتخاب کنید
query-query-sheet-parts-group-selected = گروه‌بندی انتخاب‌شده‌ها ▾
query-query-sheet-parts-clear = پاک کردن
query-query-sheet-parts-all-of = همهٔ
query-query-sheet-parts-any-of = هر یک از
query-query-sheet-parts-none-of = هیچ‌یک از
query-query-sheet-parts-from = از
query-query-sheet-parts-value = مقدار
query-query-sheet-parts-to = تا
query-query-sheet-parts-add-condition = + افزودن شرط
query-query-sheet-parts-type-to-add-a-condition = برای افزودن شرط تایپ کنید
query-query-sheet-parts-page-property-2 = { " " }(ویژگی صفحه)
query-query-sheet-parts-reading-this-graph-s-properties = در حال خواندن ویژگی‌های این گراف…
query-query-sheet-parts-try-again = تلاش دوباره
query-query-sheet-parts-add = افزودن
query-query-workspace-could-not-save = «{ $value1 }» ذخیره نشد: { $value2 }
query-query-workspace-filters-and-advanced-query = پالایه‌ها و پرس‌وجوی پیشرفته
query-query-workspace-use-the-friendly-fields-or-switch-losslessly-to-the-visual = از فیلدهای ساده استفاده کنید یا بدون از دست دادن چیزی به سازندهٔ دیداری پرس‌وجو بروید.
query-query-workspace-close-filters = بستن پالایه‌ها
query-query-workspace-switching-back-to-friendly-fields-is-offered-only-when-it = بازگشت به فیلدهای ساده فقط وقتی پیشنهاد می‌شود که چیزی از دست نرود.
query-query-workspace-this-search-uses-a-combination-that-cannot-be-split-into = این جستجو ترکیبی دارد که بدون تغییرش به فیلدها تقسیم نمی‌شود.
query-query-workspace-friendly-search-syntax = نحو سادهٔ جستجو
query-query-workspace-edit-as-visual-query = ویرایش به‌صورت پرس‌وجوی دیداری
query-query-workspace-all-of-these-words = همهٔ این کلمات
query-query-workspace-any-of-these-words = هر یک از این کلمات
query-query-workspace-this-exact-phrase = این عبارت دقیق
query-query-workspace-exclude-these-words = بدون این کلمات
query-query-workspace-case-sensitive-regular-expression = عبارت باقاعدهٔ حساس به بزرگی و کوچکی حروف
query-query-workspace-pattern-without = الگو بدون / /
query-query-workspace-couldn-t-stop-this-workspace-s-search = جستجوی این فضای کار متوقف نشد.
query-query-workspace-search-and-query-workspace = فضای کار جستجو و پرس‌وجو
query-query-workspace-search = جستجو
query-query-workspace-query-dsl = DSL پرس‌وجو
query-query-workspace-search-pages-and-blocks = جستجوی صفحه‌ها و بلوک‌ها
query-query-workspace-filters-advanced = پالایه‌ها / پیشرفته
query-query-workspace-runs-the-saved-query-expression-against-blocks-its-presentation-is = عبارت پرس‌وجوی ذخیره‌شده را روی بلوک‌ها اجرا می‌کند. نمایش آن جداگانه تعیین می‌شود.
query-query-workspace-pages-match = تطبیق صفحه‌ها
query-query-workspace-names-and-aliases = نام‌ها و نام‌های مستعار
query-query-workspace-block-content = محتوای بلوک
query-query-workspace-names-or-content = نام‌ها یا محتوا
query-query-workspace-result-presentation = نمایش نتیجه
query-query-workspace-hide-explanation = پنهان کردن توضیح
query-query-workspace-explain-query = توضیح پرس‌وجو
query-query-workspace-page-title = عنوان صفحه
query-query-workspace-name-this-search-to-save-it-as-a-page = برای ذخیره به‌صورت صفحه، این جستجو را نام‌گذاری کنید
query-query-workspace-saving = در حال ذخیره…
query-query-workspace-save-page = ذخیرهٔ صفحه
query-query-workspace-enter-a-search-to-begin = برای شروع جستجویی وارد کنید.
query-query-workspace-enter-a-query-to-begin = برای شروع پرس‌وجویی وارد کنید.
query-query-workspace-searching = در حال جستجو…
query-query-workspace-search-superseded-by-a-newer-request = جستجو با درخواست جدیدتری جایگزین شد.
query-query-workspace-query-diagnostics = عیب‌یابی پرس‌وجو
query-query-workspace-query-explanation = توضیح پرس‌وجو
query-query-workspace-how-this-query-works = این پرس‌وجو چگونه کار می‌کند
query-query-workspace-block-results = نتایج بلوک
query-query-workspace-query-results = نتایج پرس‌وجو
query-query-workspace-type = نوع
query-query-workspace-page = صفحه
query-query-workspace-content = محتوا
query-query-workspace-query-results-grouped-by-page = نتایج پرس‌وجو، گروه‌بندی‌شده با صفحه
query-query-display-move-up = انتقال { $value1 } به بالا
query-query-display-move-down = انتقال { $value1 } به پایین
query-query-display-remove = حذف { $value1 }
query-query-display-display-pages = نمایش صفحه‌ها
query-query-display-display = نمایش
query-query-display-page-display = نمایش صفحه
query-query-display-block-display = نمایش بلوک
query-query-display-view = نما
query-query-display-query-view = نمای پرس‌وجو
query-query-display-group-by = گروه‌بندی با
query-query-display-no-grouping = بدون گروه‌بندی
query-query-display-none = هیچ
query-query-display-sort = مرتب‌سازی
query-query-display-asc = صعودی
query-query-display-desc = نزولی
query-query-display-columns = ستون‌ها
query-query-display-summarize = خلاصه‌سازی
query-query-display-count = + شمارش
query-query-display-sample = نمونه
query-query-display-sample-size = اندازهٔ نمونه
query-query-display-no-limit = بدون محدودیت
query-query-display-a-sample-is-a-whole-number-up-to-4294967295 = نمونه عددی صحیح تا 4294967295 است.
query-query-display-no-results-sample-0 = بدون نتیجه (نمونهٔ ۰)
query-query-display-done = انجام شد
query-query-sheet-query-filter = پالایهٔ پرس‌وجو
query-query-sheet-try = امتحان کنید:
query-query-sheet-query-conditions = شرط‌های پرس‌وجو
query-query-sheet-conditions = شرط‌ها، { $value1 }
query-query-sheet-switch-between-all-of-any-of = جابه‌جایی میان «همهٔ» و «هر یک از»
query-query-sheet-group-actions = کنش‌های گروه
query-query-sheet-remove-none-of = حذف «هیچ‌یک از»
query-query-sheet-ungroup = لغو گروه‌بندی
query-query-sheet-remove = حذف
query-query-sheet-edit-in-the-query-text-below = در متن پرس‌وجوی پایین ویرایش کنید
query-query-sheet-edit-this-part-in-the-query-text-below = این بخش را در متن پرس‌وجوی پایین ویرایش کنید
query-query-sheet-edit-in-the-query-text = در متن پرس‌وجو ویرایش کنید: { $value1 }
query-query-sheet-remove-condition = حذف شرط
query-query-sheet-row-actions = کنش‌های ردیف
query-query-sheet-this-condition-was-not-understood = این شرط فهمیده نشد.
query-query-sheet-did-you-mean = منظورتان این بود؟
query-query-sheet-edit-as-text = ویرایش به‌صورت متن
query-query-sheet-condition-value = مقدار شرط
query-query-result-parts-edit-properties-of = ویرایش ویژگی‌های { $value1 }
query-query-result-parts-edit-page-properties = ویرایش ویژگی‌های صفحه
query-query-result-parts-page-results = نتایج صفحه
query-query-result-parts-page-results-grouped = نتایج صفحهٔ گروه‌بندی‌شده
query-query-result-parts-no-value = بدون مقدار
query-query-result-parts-a-row-with-several-tags-appears-in-every-matching-group = ردیفی با چند برچسب در هر گروه مطابق می‌آید، پس جمع این شمارش‌ها ممکن است از نتیجه بیشتر شود.
query-query-result-parts-this-query-has-no-conditions-so-it-shows-nothing-as = این پرس‌وجو شرطی ندارد، پس چیزی نشان نمی‌دهد، مانند Logseq.
query-query-result-parts-on-its-own-it-matches-nothing = به‌تنهایی با چیزی مطابق نمی‌شود.
query-query-live-preview-live-results = نتایج زنده
query-query-live-preview-running = در حال اجرا…
query-query-live-preview-page = صفحه
query-query-live-preview-nothing-matches-these-conditions = چیزی با این شرط‌ها مطابق نیست.
query-query-live-preview-more-pages-match-than-are-shown = صفحه‌های مطابق بیش از موارد نمایش‌داده‌شده است.
query-query-live-preview-more-blocks-match-than-are-shown = بلوک‌های مطابق بیش از موارد نمایش‌داده‌شده است.
query-query-builder-query-text = متن پرس‌وجو
query-query-builder-raw-query-dsl = DSL خام پرس‌وجو
query-query-builder-query-text-tql = متن پرس‌وجو (TQL)
query-query-builder-query-expression = عبارت پرس‌وجو
query-query-builder-checking = در حال بررسی…
query-query-builder-save-query-text = ذخیرهٔ متن پرس‌وجو
query-query-builder-in-a-disabled-condition-the-query-still-runs = { " " }(در شرطی غیرفعال؛ پرس‌وجو همچنان اجرا می‌شود)
query-query-builder-show-me = نشانم بده
query-query-builder-properties-in-this-graph = ویژگی‌های این گراف:{ " " }
query-filter-condition = شرط
query-filter-page-tag-reference = ارجاع صفحه / برچسب
query-filter-task-marker = نشانگر وظیفه
query-filter-priority = اولویت
query-filter-property = ویژگی
query-filter-scheduled = زمان‌بندی‌شده
query-filter-deadline = مهلت
query-filter-in-a-journal-page = در صفحهٔ روزنوشت
query-filter-between-dates = میان تاریخ‌ها
query-filter-full-text-search = جستجوی تمام‌متن
query-filter-page = صفحه
query-filter-page-property = ویژگی صفحه
query-filter-page-tags = برچسب‌های صفحه
query-filter-references = ارجاع می‌دهد به
query-filter-does-not-reference = ارجاع نمی‌دهد به
query-filter-is-any-of = هر یک از این‌هاست
query-filter-is-none-of = هیچ‌یک از این‌ها نیست
query-filter-is = برابر است با
query-filter-is-not = برابر نیست با
query-filter-is-set = تنظیم شده
query-filter-is-not-set = تنظیم نشده
query-filter-is-a-journal-page = صفحهٔ روزنوشت است
query-filter-is-not-a-journal-page = صفحهٔ روزنوشت نیست
query-filter-between = میان
query-filter-not-between = نه میان
query-filter-contains = شامل است
query-filter-does-not-contain = شامل نیست
query-operator-in-namespace = در فضای نام است
query-filter-not-in-namespace = در فضای نام نیست
query-filter-matches = مطابق است با
query-filter-does-not-match = مطابق نیست با
query-prose-open-tasks = وظایف باز
query-prose-any-task = هر وظیفه
query-prose-a-b-or-c = A، B یا C
query-prose-move-group = انتقال گروه
query-prose-select-group = انتخاب گروه
query-prose-group-actions = کنش‌های گروه
query-prose-group-enabled = گروه فعال
query-prose-move-condition = انتقال شرط
query-prose-select-condition = انتخاب شرط
query-prose-condition-enabled = شرط فعال
query-prose-row-actions = کنش‌های ردیف
query-prose-condition-operator = عملگر شرط
query-prose-page-property = ویژگی صفحه
query-prose-property = ویژگی
query-prose-not = نه
query-prose-matches = مطابق است با
query-prose-reading-this-graph-s-properties = در حال خواندن ویژگی‌های این گراف…
query-prose-type-to-search-page-names-and-block-text = برای جستجوی نام صفحه‌ها و متن بلوک‌ها تایپ کنید.
query-prose-page = صفحه
query-prose-block = بلوک
query-prose-what-this-query-selects = این پرس‌وجو چه چیزی را انتخاب می‌کند
query-prose-group-the-selected-conditions = گروه‌بندی شرط‌های انتخاب‌شده
query-prose-disabled-by-group = غیرفعال با گروه
query-prose-disabled = غیرفعال
query-prose-journal-date = تاریخ روزنوشت
query-prose-scheduled = زمان‌بندی‌شده
query-prose-deadline = مهلت
query-prose-any-date = هر تاریخ
query-prose-count = شمارش
query-prose-sum = جمع
query-prose-avg = میانگین
query-prose-advanced = ⟨پیشرفته⟩
query-prose-today = امروز
query-prose-yesterday = دیروز
query-prose-tomorrow = فردا
query-prose-journal-date-field = تاریخ روزنوشت
query-prose-not-group = نه
query-prose-everything = همه‌چیز
query-prose-nothing = هیچ‌چیز
query-prose-on-journal = در صفحهٔ روزنوشت
query-prose-any-status = هر وضعیت
query-prose-no-markers = بدون نشانگر
query-prose-no-levels = بدون سطح
query-prose-field-value = { $field }: { $value }
query-prose-on-page = صفحه: { $value }
query-prose-in-namespace = فضای نام: { $value }
query-prose-page-property-clause = صفحه { $inner }
query-prose-page-tags = برچسب‌های صفحه: { $value }
query-prose-property-any = { $field }: هر مقدار
query-prose-task = وظیفه: { $value }
query-prose-priority = اولویت: { $value }
query-prose-content = متن: «{ $value }»
query-prose-search = جستجو: { $value }
query-prose-date-range = { $low } تا { $high }
query-prose-off = { $inner } (خاموش)
query-prose-negation = نه { $inner }
query-prose-parentheses = ({ $inner })
query-prose-sentence = { $subject } که در آن { $predicate }
query-prose-attr-content = متن
query-prose-attr-task = وظیفه
query-prose-attr-priority = اولویت
query-prose-attr-scheduled = زمان‌بندی
query-prose-attr-deadline = مهلت
query-prose-attr-created-at = ساخته‌شده
query-prose-attr-last-modified-at = آخرین تغییر
query-prose-attr-name = نام
query-prose-attr-journal = روزنوشت
query-prose-attr-day = تاریخ
query-prose-attr-namespace = فضای نام
query-prose-attr-used-as-tag = به‌کاررفته به‌عنوان برچسب
query-prose-attr-key = کلید
query-prose-attr-value = مقدار
query-prose-attr-atom-count = مقادیر
query-prose-property-operator =
    { $op ->
       *[is] { $cardinality ->
                [list] شامل این مقدار است
               *[scalar] برابر است با
            }
        [is_not] { $cardinality ->
                [list] شامل این مقدار نیست
               *[scalar] برابر نیست با
            }
        [gt] بیشتر است از
        [ge] دست‌کم است
        [lt] کمتر است از
        [le] حداکثر است
        [between] میان است
        [before] پیش از
        [on_or_before] تا
        [after] پس از
        [on_or_after] از
        [contains] شامل است
        [does_not_contain] شامل نیست
        [starts_with] شروع می‌شود با
        [ends_with] پایان می‌یابد با
        [references] ارجاع می‌دهد به
        [does_not_reference] ارجاع نمی‌دهد به
        [is_checked] تیک خورده
        [is_unchecked] تیک نخورده
        [is_set] تنظیم شده
        [is_not_set] تنظیم نشده
        [is_blank] خالی است
        [has_other_value] مقداری غیر از این دارد
    }
query-prose-property-test =
    { $op ->
       *[is] { $field } برابر است با { $value }
        [is_not] { $field } برابر نیست با { $value }
        [gt] { $field } بیشتر است از { $value }
        [ge] { $field } دست‌کم { $value } است
        [lt] { $field } کمتر است از { $value }
        [le] { $field } حداکثر { $value } است
        [between] { $field } میان { $value } است
        [before] { $field } پیش از { $value }
        [on_or_before] { $field } تا { $value }
        [after] { $field } پس از { $value }
        [on_or_after] { $field } از { $value }
        [contains] { $field } شامل { $value } است
        [does_not_contain] { $field } شامل { $value } نیست
        [starts_with] { $field } با { $value } شروع می‌شود
        [ends_with] { $field } با { $value } پایان می‌یابد
        [references] { $field } به { $value } ارجاع می‌دهد
        [does_not_reference] { $field } به { $value } ارجاع نمی‌دهد
        [is_checked] { $field } تیک خورده
        [is_unchecked] { $field } تیک نخورده
        [is_set] { $field } تنظیم شده
        [is_not_set] { $field } تنظیم نشده
        [is_blank] { $field } خالی است
        [has_other_value] { $field } مقداری غیر از { $value } دارد
    }
query-prose-attribute-test =
    { $op ->
       *[eq] { $field } برابر است با { $value }
        [not_eq] { $field } برابر نیست با { $value }
        [lt] { $field } < { $value }
        [le] { $field } ≤ { $value }
        [gt] { $field } > { $value }
        [ge] { $field } ≥ { $value }
        [between] { $field } میان { $value }
        [in] { $field } یکی از { $value } است
        [not_in] { $field } هیچ‌یک از { $value } نیست
        [like] { $field } شامل { $value } است
        [starts_with] { $field } با { $value } شروع می‌شود
        [match] { $field } مطابق است با { $value }
        [regex] { $field } مطابق عبارت باقاعدهٔ { $value } است
        [is_set] { $field } تنظیم شده
        [is_not_set] { $field } تنظیم نشده
        [is_blank] { $field } خالی است
    }
query-prose-boolean =
    { $value ->
        [true] بله
       *[false] خیر
    }
query-prose-group =
    { $kind ->
       *[and] و
        [or] یا
    }
query-prose-subject =
    { $family ->
        [both] صفحه‌ها و بلوک‌ها
        [page] صفحه‌ها
       *[block] بلوک‌ها
    }
query-prose-all =
    { $family ->
        [both] همهٔ صفحه‌ها و بلوک‌ها
        [page] همهٔ صفحه‌ها
       *[block] همهٔ بلوک‌ها
    }
query-prose-date-bound =
    { $op ->
       *[ge] از { $when }
        [le] تا { $when }
    }
query-prose-relative-days = { NUMBER($count, useGrouping: 0) } روز { $direction ->
        [ago] پیش
       *[ahead] بعد
    }
query-prose-relative-weeks = { NUMBER($count, useGrouping: 0) } هفته { $direction ->
        [ago] پیش
       *[ahead] بعد
    }
query-prose-relative-months = { NUMBER($count, useGrouping: 0) } ماه { $direction ->
        [ago] پیش
       *[ahead] بعد
    }
query-prose-relative-years = { NUMBER($count, useGrouping: 0) } سال { $direction ->
        [ago] پیش
       *[ahead] بعد
    }
query-prose-list = { $left }{ $position ->
        [middle] { "، " }
        [last-many] { $connector ->
            [and] { " و " }
           *[or] { " یا " }
        }
       *[last] { $connector ->
            [and] { " و " }
           *[or] { " یا " }
        }
    }{ $right }
query-prose-relation = { $quant } { $relation }: { $inner }
query-prose-not-symbol = نه { $symbol }
query-prose-invalid-regex = عبارت باقاعده نامعتبر است: { $error }
query-prose-regex-summary = نام صفحه‌ها یا متن بلوک‌ها را با عبارت باقاعدهٔ حساس به بزرگی و کوچکی /{ $pattern }/ تطبیق می‌دهد.
query-prose-search-summary = نام صفحه‌ها یا متن بلوک‌ها را تطبیق می‌دهد { $group }.
query-prose-search-groups-summary = نام صفحه‌ها یا متن بلوک‌ها را تطبیق می‌دهد وقتی { $groups }.
query-prose-search-and = { $left } و { $right }
query-prose-search-or = { $left }؛ یا { $right }
query-prose-search-term = { $kind ->
        [exclude-exact] بدون عبارت دقیق «{ $value }»
        [exclude] بدون «{ $value }»
        [exact] شامل عبارت دقیق «{ $value }»
       *[contains] شامل «{ $value }»
    }
query-prose-aggregate-field = { $verb } { $field }
query-prose-advanced-clause = { $advanced } { $clause }
query-prose-aggregate-unavailable = ناموجود ({ $reason ->
        [non_finite] نامتناهی
        [division_by_zero] تقسیم بر صفر
        [empty_group] گروه خالی
       *[non_numeric] غیرعددی
    })
query-prose-formula-statistics-notice = آمار دقیق بر پایهٔ فرمول هنوز پشتیبانی نمی‌شود. آمار کلی نمایش داده می‌شود.
query-export-dialog-left-out = { $count } صفحهٔ انتخاب‌شده شامل نشده است:
query-export-dialog-left-out-private = با public:: false علامت خورده
query-export-dialog-left-out-exports-page = صفحهٔ خروجی‌ها هرگز خروجی گرفته نمی‌شود
query-export-dialog-selects-host-page = این شامل صفحه‌ای است که پرس‌وجو در آن نوشته شده.
query-export-dialog-include-query-text = شامل کردن متن پرس‌وجو (خوانندگان پرس‌وجو را هم می‌بینند، نه فقط صفحه‌ها)
