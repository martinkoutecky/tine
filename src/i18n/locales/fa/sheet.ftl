sheet-can-t-convert-to-grid-children-would-collide-with-grid-rows = تبدیل به شبکه ممکن نیست: فرزندان با ردیف‌های شبکه برخورد می‌کنند.
sheet-can-t-convert-this-parser-does-not-round-trip-inside-table-cells = تبدیل ممکن نیست: این تجزیه‌گر \| درون خانه‌های جدول را درست بازنویسی نمی‌کند.
sheet-couldn-t-cut-cells-clipboard-write-failed = خانه‌ها بریده نشدند: نوشتن در کلیپ‌بورد ناموفق بود.
sheet-pasted-over-existing-cells = روی خانه‌های موجود جای‌گذاری شد.
sheet-undo = واگرد
sheet-pasted-outline-is-too-deep = طرح کلی جای‌گذاری‌شده بیش از حد عمیق است
sheet-couldn-t-copy-cells-clipboard-write-failed = خانه‌ها کپی نشدند: نوشتن در کلیپ‌بورد ناموفق بود.
sheet-resize-column = تغییر اندازهٔ ستون { $label }
sheet-drag-to-resize-double-click-to-reset = برای تغییر اندازه بکشید؛ دوبار کلیک برای بازنشانی
sheet-grid-requires-markdown = تبدیل به جدول Markdown فقط برای شبکه‌های markdown در دسترس است.
sheet-grid-read-only = شبکهٔ فقط‌خواندنی به جدول Markdown تبدیل نمی‌شود.
sheet-grid-title-required = تبدیل ممکن نیست: بلوک شبکه به خط عنوان بالای ویژگی‌های tine.* نیاز دارد.
sheet-grid-too-large = شبکه‌های بزرگ‌تر از ۳۰ ستون در ۲۰۰ ردیف به جدول Markdown تبدیل نمی‌شوند.
sheet-grid-empty = شبکهٔ خالی به جدول Markdown تبدیل نمی‌شود.
sheet-grid-row-missing = این شبکه تبدیل نمی‌شود چون یک ردیف کم است.
sheet-grid-row-properties = تبدیل ممکن نیست: ویژگی‌های پنهان ردیف از بین می‌روند.
sheet-grid-row-content = تبدیل ممکن نیست: بلوک‌های ردیف باید خالی باشند.
sheet-grid-row-empty = تبدیل ممکن نیست: ردیف‌های خالی در جدول Markdown نمایش‌پذیر نیستند.
sheet-grid-cell-missing = این شبکه تبدیل نمی‌شود چون یک خانه کم است.
sheet-grid-cell-properties = تبدیل ممکن نیست: ویژگی‌های پنهان خانه از بین می‌روند.
sheet-grid-cell-children = تبدیل ممکن نیست: خانه‌های فرزنددار در جدول Markdown نمایش‌پذیر نیستند.
sheet-grid-cell-multiline = تبدیل ممکن نیست: خانه‌ها باید تک‌خطی باشند.
sheet-grid-cell-pipe = تبدیل ممکن نیست: این تجزیه‌گر \| درون خانه‌های جدول را درست بازنویسی نمی‌کند.
sheet-load-more = بارگذاری { NUMBER($count, useGrouping: 0) } { $kind ->
        [cards] کارت
       *[other] ردیف
    } دیگر ({ NUMBER($shown, useGrouping: 0) } از { NUMBER($total, useGrouping: 0) })
sheet-rename-field =
    { $hasReason ->
        [1] تغییر نام فیلد… ({ $reason })
       *[other] تغییر نام فیلد…
    }
sheet-cell-refused = آن خانه{ $hasValue ->
        [1] { " " }به «{ $value }»
       *[other] { "" }
    } تغییر نکرد: صفحه فقط‌خواندنی است، ردیف از بین رفته، یا مقدار با ستون سازگار نیست.
sheet-type-text = متن
sheet-type-number = عدد
sheet-type-date = تاریخ
sheet-type-datetime = تاریخ و ساعت
sheet-type-checkbox = چک‌باکس
sheet-type-list = فهرست
sheet-type-ref = ارجاع
sheet-sheet-table-this-order-cannot-be-saved-while-a-computed-column-is = تا وقتی ستون محاسبه‌شده نمایان است، این ترتیب ذخیره نمی‌شود.
sheet-sheet-table-some-direct-rows-are-not-loaded-no-fields-were-renamed = برخی ردیف‌های مستقیم بارگذاری نشده‌اند؛ نام هیچ فیلدی تغییر نکرد.
sheet-sheet-table-rename-field-formula-columns-cannot-be-renamed-here = تغییر نام فیلد… (نام ستون‌های فرمولی اینجا تغییر نمی‌کند)
sheet-sheet-table-rename-field-only-children-backed-tables-can-rename-fields = تغییر نام فیلد… (فقط جدول‌های مبتنی بر فرزندان)
sheet-sheet-table-edit-formula = ویرایش فرمول…
sheet-sheet-table-remove-formula = حذف فرمول
sheet-sheet-table-add-formula = افزودن فرمول…
sheet-sheet-table-declare-field-text = تعریف فیلد (متن)
sheet-sheet-table-type = نوع ←
sheet-sheet-table-remove-from-schema = حذف از طرح داده
sheet-sheet-table-remove-column = حذف ستون
sheet-sheet-table-empty-table = جدول خالی
sheet-sheet-table-add-row = افزودن ردیف
sheet-sheet-table-block = بلوک
sheet-sheet-table-clear-table-only-sort = پاک کردن مرتب‌سازی جدول
sheet-sheet-table-filter-disabled = پالایه غیرفعال است
sheet-sheet-table-rename-field-3 = تغییر نام فیلد { $value1 }
sheet-sheet-table-add-column = افزودن ستون
sheet-sheet-table-property = ویژگی
sheet-sheet-table-cell-menu = منوی خانه
sheet-sheet-table-clear = پاک کردن
sheet-sheet-board-moved-out-of-this-query-s-results = از نتایج این پرس‌وجو بیرون رفت
sheet-sheet-board-all-results = همهٔ نتایج
sheet-sheet-board-invalid-tag-name = نام برچسب نامعتبر است
sheet-sheet-board-group-by = گروه‌بندی بر اساس
sheet-sheet-board-no-grouping = بدون گروه‌بندی
sheet-sheet-board-empty-board = تختهٔ خالی
sheet-sheet-board-add-tag-column = افزودن ستون برچسب
sheet-sheet-board-new-tag = برچسب جدید
sheet-sheet-board-new-tag-2 = new-tag
sheet-sheet-board-card-menu = منوی کارت
sheet-prose-state = وضعیت
sheet-prose-priority = اولویت
sheet-prose-scheduled = زمان‌بندی
sheet-prose-deadline = مهلت
sheet-prose-tags = برچسب‌ها
sheet-prose-page = صفحه
sheet-prose-table-only-sort = مرتب‌سازی فقط جدول: { $field }
sheet-prose-title = عنوان
sheet-board-empty-column = (هیچ)
sheet-aggregate-none = هیچ
sheet-aggregate-name = { $fn ->
    [sum] جمع
    [average] میانگین
    [median] میانه
    [min] کمینه
    [max] بیشینه
    [range] دامنه
    [stddev] انحراف معیار
    [earliest] زودترین
    [latest] دیرترین
    [empty] خالی
    [filled] پر
    [unique] یکتا
    [checked] تیک‌خورده
    [unchecked] بدون تیک
   *[count] شمار
    }
