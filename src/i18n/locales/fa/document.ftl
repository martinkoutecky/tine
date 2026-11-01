document-review-unsaved = بررسی موارد ذخیره‌نشده
document-create-unreadable = «{ $name }» ساخته نشد: { $hasOwner ->
        [1] { $owner }
       *[other] یک فایل صفحه
    } خوانده نمی‌شود و شاید همین صفحه باشد. آن فایل را اصلاح یا جابه‌جا کنید؛ ویرایش‌های شما در ویرایشگر می‌مانند.
document-read-refused = «{ $name }» از دیسک خوانده نشد؛ ویرایش‌های شما در ویرایشگر می‌مانند.
document-draft-refused = نسخهٔ بازیابی پس از خرابی برای «{ $name }» نگه داشته نشد؛ ویرایش‌های شما در ویرایشگر می‌مانند.
document-save-refused = «{ $name }» ذخیره نشد: { $detail }
document-save-twin = «{ $name }» ذخیره نشد: { $existing } از قبل همین صفحه است. ویرایش‌های شما در ویرایشگر می‌مانند.
document-save-undecodable = «{ $name }» ذخیره نشد: فایلش متن خواندنی نیست. ویرایش‌های شما در ویرایشگر می‌مانند.
document-save-retrying = «{ $name }» هنوز ذخیره نشده{ $step }؛ Tine به تلاش ادامه می‌دهد و نسخهٔ بازیابی نگه می‌دارد.
document-operation-unrecorded = تغییر نام یا حذف «{ $name }» که Tine نتوانست انجام نشدنش را تأیید کند، ثبت نشد. چیزی تغییر نکرد.
document-draft-unwritten = نسخهٔ بازیابی «{ $name }» نوشته نشد.
document-load-refused = «{ $page }» از { $hasHolder ->
        [1] { $holder }
       *[other] صفحه‌ای جدید که هنوز ذخیره نشده
    } با کار ذخیره‌نشده باز است، پس { $hasRequested ->
        [1] { $requested }
       *[other] صفحه
    } بارگذاری نشد. ابتدا آن ویرایش را تمام یا ذخیره کنید.{ $hasAfter ->
        [1] { " " }{ $after }
       *[other] { "" }
    }
document-move-incomplete =
    { $completed ->
        [0] جابه‌جایی انجام نشد: «{ $pages }» ابتدا ذخیره نشد.
       *[other] فقط بخشی از جابه‌جایی انجام شد: بلوک‌های «{ $pages }» جابه‌جا نشدند.
    }
document-external-updates = { NUMBER($pages, useGrouping: 0) } صفحه از بیرون به‌روز شد{ $conflicts ->
        [0] { "" }
       *[other] { " " }· { NUMBER($conflicts, useGrouping: 0) } تعارض برای بررسی
    }
document-wiring-tine-can-t-save-this-graph = Tine نمی‌تواند این گراف را ذخیره کند: { $value1 }
document-wiring-saved-the-page-but-could-not-refresh-its-title = صفحه ذخیره شد، اما عنوانش تازه نشد: { $value1 }
document-wiring-saved-the-title-but-its-page-identity-could-not-be = عنوان ذخیره شد، اما هویت صفحه به‌طور امن پذیرفته نشد. این صفحه را با مسیر فایلش دوباره باز کنید.
document-wiring-resolve-the-conflict-on-first = ابتدا تعارض «{ $value1 }» را حل کنید.
document-wiring-changed-meanwhile-choose-again = «{ $value1 }» در این میان تغییر کرد. دوباره انتخاب کنید.
document-wiring-couldn-t-overwrite = «{ $value1 }» بازنویسی نشد.
document-wiring-couldn-t-read-the-page-is-an-alias-of-its = { $value1 } خوانده نشد، صفحه‌ای که «{ $value2 }» نام مستعار آن است؛ متنش اینجا ذخیره‌نشده می‌ماند.
document-wiring-changed-since-was-added-to-it-copy-the-rest-of = «{ $value1 }» پس از افزوده شدن «{ $value2 }» تغییر کرده است؛ بقیهٔ «{ $value3 }» را دستی کپی کنید.
document-wiring-moved-into-its-alias-owner = «{ $value1 }» به صاحب نام مستعارش «{ $value2 }» منتقل شد.
document-transfer-rolled-back = جابه‌جایی ذخیره نشد. بلوک‌ها و ورودی صفحه بازگردانده شدند.
