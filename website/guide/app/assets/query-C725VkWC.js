const e=`query-text-vocabulary-guide = 此图谱的字段和属性列在上方选择器中；查询语言说明位于指南的 <em>Find and revisit → Query text (TQL)</em>。

query-registry-failed = 无法读取此图谱的属性。{ $detail }

query-anchor-unchanged = 查询对象未更改：{ $detail }

query-grouped-by = 按 { $field } 分组

query-skipped-non-numeric = { " " }（已跳过 { NUMBER($count, useGrouping: 0) } 个非数字值）

query-skipped = { " " }（已跳过 { NUMBER($count, useGrouping: 0) } 项）

query-og-bare-task = <code>{ $syntax }</code> 未指定标记，不添加条件，与 Logseq 一致。{ " " }

query-og-bare-priority = <code>{ $syntax }</code> 未指定级别，不添加条件，与 Logseq 一致。{ " " }

query-preview-load-failed = 无法加载结果：{ $detail }

query-preview-count = { $kind ->
    [page] { $count ->
        [one] { NUMBER($count, useGrouping: 0) } 个页面
       *[other] { NUMBER($count, useGrouping: 0) } 个页面
        }
   *[other] { $count ->
        [one] { NUMBER($count, useGrouping: 0) } 项结果
       *[other] { NUMBER($count, useGrouping: 0) } 项结果
        }
    }

query-preview-limit = { " " }（显示前 { NUMBER($count, useGrouping: 0) } 项）

query-preview-families = { $pages ->
    [one] { NUMBER($pages, useGrouping: 0) } 个页面
   *[other] { NUMBER($pages, useGrouping: 0) } 个页面
    } · { $blocks ->
    [one] { NUMBER($blocks, useGrouping: 0) } 个块
   *[other] { NUMBER($blocks, useGrouping: 0) } 个块
    }

query-preview-family-limit = { " " }（各显示前 { NUMBER($count, useGrouping: 0) } 项）

query-preview-pages-note = 页面：{ $detail }

query-preview-blocks-note = 块：{ $detail }

query-workspace-result-count = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } 项结果
   *[other] { NUMBER($count, useGrouping: 0) } 项结果
    }

query-workspace-search-failed = 搜索失败：{ $detail }

query-workspace-explanation = { $hasWithout ->
    [1] { $conjunct }：单独使用匹配 { NUMBER($alone, useGrouping: 0) } 项，去除此条件匹配 { NUMBER($without, useGrouping: 0) } 项
   *[other] { $conjunct }：单独使用匹配 { NUMBER($alone, useGrouping: 0) } 项
    }

query-selected-count = 已选 { NUMBER($count, useGrouping: 0) } 项

query-anchor-label = { $anchor ->
    [page] 页面
    [both] 页面和块
   *[other] 块
    }

query-anchor-switch = 切换到<strong>{ query-anchor-label }</strong>：{ NUMBER($total, useGrouping: 0) } 个条件中有 { NUMBER($count, useGrouping: 0) } 个不适用于{ query-anchor-label }{ $hasNames ->
    [1] { " " }（<code>{ $names }</code>）
   *[other] { "" }
    }。

query-display-change = 更改

query-sort-direction = { $field } 排序改为{ $direction ->
    [asc] 降序
   *[other] 升序
    }

query-display-add-sort = + 排序

query-display-add-column = + 列

query-display-add-property = + 属性

query-display-whole-result = 全部结果

query-search-positive-term = 请至少添加一个要包含的搜索词。
query-unsupported-clause = 暂不支持此查询子句：{ $clause }
query-fields-regex-warning = 正则表达式暂不能与其他便捷字段组合使用。
query-options-map-warning = 请使用标题和显示控件编辑选项映射（title、collapsed），而不是在此处编辑。
query-save-superseded = 保存期间此工作区已更改，因此未写入任何内容。请重新保存。
query-save-enter-title = 保存前请输入页面标题。
query-save-enter-query = 保存前请输入搜索或查询。
query-save-graph-changed = 保存此工作区前图谱已切换。
query-save-validation-superseded = 搜索验证已被新的请求取代。请重新保存。
query-save-included-term = 保存前请输入至少包含一个搜索词的搜索。
query-save-validation-detail = 无法验证此搜索：{ $detail }
query-save-validation-failed = 无法验证此搜索。
query-save-title-exists = 名为“{ $name }”的页面已存在。请使用其他标题。
query-save-title-alias = “{ $name }”是已有页面的别名。请使用其他标题。
query-save-local-state-changed = “{ $name }”的本地页面状态已更改，无法保存。请重试。
query-save-conflict = 保存此工作区前“{ $name }”已被创建或更改。该页面未被覆盖。
query-save-failed-detail = 无法保存“{ $name }”：{ $detail }
query-save-failed = 无法保存“{ $name }”。
query-fields-quotation-warning = 这些字段中不支持引号。
query-fields-include-warning = 请至少添加一个要包含的词或完整短语。
query-dsl-empty-warning = 查询 DSL 不能为空。
query-save-earlier-search = 已根据先前的搜索保存“{ $name }”，此工作区保持原样。
query-workspace-view-search = 搜索
query-workspace-view-list = 列表
query-workspace-view-table = 表格
query-workspace-view-board = 看板

query-display-view-search = 搜索

query-display-view-list = 列表

query-display-view-table = 表格

query-display-view-board = 看板

query-display-aggregate-count = 计数

query-display-aggregate-sum = 求和

query-display-aggregate-avg = 平均值

query-export-dialog-export-query-results = 导出查询结果

query-export-dialog-name = 名称

query-export-dialog-resolving-pages = 正在解析页面…

query-export-dialog-journal = { " " }· 日记

query-export-dialog-all-blocks-on-these-pages-will-be-exported = 这些页面上的所有块都将导出，包括未匹配的块。

query-export-dialog-destination = 目标位置

query-export-dialog-replace-it-the-previous-export-is-kept-in = { " " }替换现有导出（旧导出保留在恢复区中）

query-export-dialog-adjust-limit-in-settings = 在设置中调整上限…

query-export-dialog-cancel = 取消

query-export-dialog-exporting = 正在导出…

query-export-dialog-export = 导出

query-export-limit-setting-query-export-size-limit = 查询导出大小上限

query-export-limit-setting-query-export-size-limit-mi-b = 查询导出大小上限（MiB）

query-export-limit-setting-default-1-gi-b = 默认：1 GiB

query-export-limit-setting-mi-b = { " " }MiB

query-export-limit-setting-reset = 重置

query-export-limit-setting-default-1-gi-b-copied-assets-must-fit = 默认：1 GiB。复制的附件必须在此上限内，超出后导出将停止。保存在本设备上。

query-legacy-table-content = 内容

query-legacy-table-page = 页面

query-listbox-type-to-filter = 输入以筛选

query-page-results-page = 页面

query-page-results-journal = 日记

query-page-results-page-label = 页面

query-page-results-page-results = 页面结果

query-page-results-page-results-grouped = 已分组的页面结果

query-page-results-no-value = 无值

query-result-sections-pages = 页面

query-result-sections-blocks = 块

query-result-sections-searching = 正在搜索…

query-result-sections-no-matching-pages = 没有匹配的页面。

query-result-sections-no-matching-blocks = 没有匹配的块。

query-vocabulary-picker-condition-field = 条件字段

query-vocabulary-picker-search-fields-and-properties = 搜索字段和属性

query-vocabulary-picker-reading-this-graph-s-properties = 正在读取此图谱的属性…

query-vocabulary-picker-try-again = 重试

query-vocabulary-picker-declared = { $declared }（已声明）

sheet-aggregate-footer-hide-aggregate-footer = 隐藏汇总行

sheet-aggregate-footer-show-aggregate-footer = 显示汇总行

sheet-aggregate-footer-add-aggregate = 添加汇总

sheet-aggregate-footer-change-aggregate = 更改汇总（{ $active_label }）

sheet-grid-show-previous-rows = 显示前面的行

sheet-grid-show-previous-columns = 显示前面的列

sheet-grid-add-column = 添加列

sheet-grid-add-row = 添加行

sheet-grid-cell-menu = 单元格菜单

crossing-notice-warning = { $has_feature ->
    [1] 此查询现已使用 Logseq 无法读取的 Tine 功能（<code>{ $feature }</code>）。Logseq 会将该块显示为纯文本。
   *[other] 此查询现已使用 Logseq 无法读取的 Tine 功能。Logseq 会将该块显示为纯文本。
    }

crossing-notice-changed-block = 块内容现为：<code>{ $text }</code>

query-export-dialog-summary = { $results } 项结果，来自 { $pages } 个完整页面

query-export-dialog-existing = 名为 <code>{ $folder }</code> 的导出已存在。

query-export-dialog-separate = 另建导出：<code>{ $folder }</code>

query-export-dialog-path = 目标位置：<code>{ $path }</code>。将这些页面导出为静态网站和只读应用；标记为 public:: false 的页面不会导出。不会上传，不包含未保存的编辑。

query-page-results-alias = 匹配别名 { $alias }

query-result-sections-more-matches = { $kind ->
    [page] 还有更多匹配的页面未显示。
   *[other] 还有更多匹配的块未显示。
    }

query-vocabulary-picker-registry-failed = 无法读取此图谱的属性。{ $detail }

property-type-mismatches = { $count } 个块或页面不符合类型

sheet-grid-more-rows = { $load ->
    [1] 加载更多行（{ $start }-{ $end } / { $total }）
   *[other] 显示后面的行（{ $start }-{ $end } / { $total }）
    }

sheet-grid-more-columns = { $load ->
    [1] 加载更多列（{ $start }-{ $end } / { $total }）
   *[other] 显示后面的列（{ $start }-{ $end } / { $total }）
    }

query-vocabulary-picker-use-property = { $unit ->
    [pages] 将“{ $key }”用作页面属性
   *[other] 将“{ $key }”用作块属性
    }

query-date-window-refusal = Logseq 在此查询形式中忽略 scheduled/deadline 日期范围，因此无法在此按原文运行此查询。转换为 Tine 查询可保留日期范围的含义。

query-convert = 转换

query-undo-conversion = 撤销转换

query-query-sheet-parts-click-to-edit-this-query = 点击编辑此查询

query-query-sheet-parts-edit-query-filter = 编辑查询筛选条件

query-query-sheet-parts-page-tag-reference = 页面 / 标签引用

query-query-sheet-parts-task-marker = 任务标记

query-query-sheet-parts-priority = 优先级

query-query-sheet-parts-property = 属性

query-query-sheet-parts-scheduled = 计划日期

query-query-sheet-parts-deadline = 截止日期

query-query-sheet-parts-in-a-journal-page = 位于日记页面

query-query-sheet-parts-between-dates = 日期范围

query-query-sheet-parts-full-text-search = 全文搜索

query-query-sheet-parts-on-page = 所在页面

query-query-sheet-parts-in-namespace = 所在命名空间

query-query-sheet-parts-page-property = 页面属性

query-query-sheet-parts-page-tags = 页面标签

query-query-sheet-parts-drag-to-reorder-or-into-another-group-or-use-the = 拖动调整顺序或移入其他分组，也可使用上下方向键

query-query-sheet-parts-the-group-above-is-disabled-so-this-does-not-run = 上级分组已禁用，因此此条件不执行

query-query-sheet-parts-disabled-kept-in-the-query-not-run = 已禁用：保留在查询中，不执行

query-query-sheet-parts-running-press-to-disable-without-removing-it = 已启用：点击可禁用而不移除

query-query-sheet-parts-move-up = 上移

query-query-sheet-parts-move-down = 下移

query-query-sheet-parts-group-with-row-above-all-of = 与上行分组：全部满足

query-query-sheet-parts-group-with-row-above-any-of = 与上行分组：满足任意

query-query-sheet-parts-group-with-row-above-none-of = 与上行分组：全部不满足

query-query-sheet-parts-apply = 应用

query-query-sheet-parts-start-today-30d-2026-06-01-or-a-page = 开始：today、-30d、2026-06-01 或页面

query-query-sheet-parts-end-today-7d-2026-06-30-or-a-page = 结束：today、+7d、2026-06-30 或页面

query-query-sheet-parts-page-or-tag-name = 页面或标签名称

query-query-sheet-parts-any-status = 任意状态

query-query-sheet-parts-page-name = 页面名称

query-query-sheet-parts-namespace-parent-page = 命名空间（父页面）

query-query-sheet-parts-text-to-search-for = 要搜索的文本

query-query-sheet-parts-search-words-and-operators = 搜索词和运算符

query-query-sheet-parts-tag-one = 标签（单个）

query-query-sheet-parts-blocks = 块

query-query-sheet-parts-individual-bullets-anywhere-in-the-graph = 图谱中任意位置的单个块

query-query-sheet-parts-pages = 页面

query-query-sheet-parts-whole-pages = 完整页面

query-query-sheet-parts-pages-and-blocks = 页面和块

query-query-sheet-parts-pages-above-matching-blocks-below = 上方为页面，下方为匹配的块

query-query-sheet-parts-find = 查找

query-query-sheet-parts-what-this-query-selects = 此查询选择的对象

query-query-sheet-parts-where = 条件…

query-query-sheet-parts-switching-what-this-query-selects = 切换此查询选择的对象

query-query-sheet-parts-remove-them = 移除这些条件

query-query-sheet-parts-keep-anyway = 仍然保留

query-query-sheet-parts-cancel = 取消

query-query-sheet-parts-selected-conditions = 所选条件

query-query-sheet-parts-select-two-or-more-conditions-to-group-them = 选择两个或更多条件以分组

query-query-sheet-parts-group-selected = 将所选条件分组 ▾

query-query-sheet-parts-clear = 清除

query-query-sheet-parts-all-of = 全部满足

query-query-sheet-parts-any-of = 满足任意

query-query-sheet-parts-none-of = 全部不满足

query-query-sheet-parts-from = 从

query-query-sheet-parts-value = 值

query-query-sheet-parts-to = 到

query-query-sheet-parts-add-condition = + 添加条件

query-query-sheet-parts-type-to-add-a-condition = 输入以添加条件

query-query-sheet-parts-page-property-2 = { " " }（页面属性）

query-query-sheet-parts-reading-this-graph-s-properties = 正在读取此图谱的属性…

query-query-sheet-parts-try-again = 重试

query-query-sheet-parts-add = 添加

query-query-workspace-could-not-save = 无法保存“{ $value1 }”：{ $value2 }

query-query-workspace-filters-and-advanced-query = 筛选与高级查询

query-query-workspace-use-the-friendly-fields-or-switch-losslessly-to-the-visual = 使用便捷字段，或无损切换到可视化查询编辑器。

query-query-workspace-close-filters = 关闭筛选

query-query-workspace-switching-back-to-friendly-fields-is-offered-only-when-it = 仅当能无损转换时，才提供切回便捷字段的选项。

query-query-workspace-this-search-uses-a-combination-that-cannot-be-split-into = 此搜索使用的条件组合无法在保持含义的情况下拆分为字段。

query-query-workspace-friendly-search-syntax = 便捷搜索语法

query-query-workspace-edit-as-visual-query = 以可视化查询编辑

query-query-workspace-all-of-these-words = 包含所有词

query-query-workspace-any-of-these-words = 包含任意词

query-query-workspace-this-exact-phrase = 包含完整短语

query-query-workspace-exclude-these-words = 排除这些词

query-query-workspace-case-sensitive-regular-expression = 区分大小写的正则表达式

query-query-workspace-pattern-without = 表达式，不含 / /

query-query-workspace-couldn-t-stop-this-workspace-s-search = 无法停止此工作区的搜索。

query-query-workspace-search-and-query-workspace = 搜索与查询工作区

query-query-workspace-search = 搜索

query-query-workspace-query-dsl = 查询 DSL

query-query-workspace-search-pages-and-blocks = 搜索页面和块

query-query-workspace-filters-advanced = 筛选 / 高级

query-query-workspace-runs-the-saved-query-expression-against-blocks-its-presentation-is = 对块运行已保存的查询表达式。显示方式单独设置。

query-query-workspace-pages-match = 页面匹配方式

query-query-workspace-names-and-aliases = 名称和别名

query-query-workspace-block-content = 块内容

query-query-workspace-names-or-content = 名称或内容

query-query-workspace-result-presentation = 结果显示方式

query-query-workspace-hide-explanation = 隐藏解释

query-query-workspace-explain-query = 解释查询

query-query-workspace-page-title = 页面标题

query-query-workspace-name-this-search-to-save-it-as-a-page = 为此搜索命名以保存为页面

query-query-workspace-saving = 正在保存…

query-query-workspace-save-page = 保存页面

query-query-workspace-enter-a-search-to-begin = 输入搜索内容以开始。

query-query-workspace-enter-a-query-to-begin = 输入查询以开始。

query-query-workspace-searching = 正在搜索…

query-query-workspace-search-superseded-by-a-newer-request = 搜索已被更新的请求取代。

query-query-workspace-query-diagnostics = 查询诊断

query-query-workspace-query-explanation = 查询解释

query-query-workspace-how-this-query-works = 此查询如何工作

query-query-workspace-block-results = 块结果

query-query-workspace-query-results = 查询结果

query-query-workspace-type = 类型

query-query-workspace-page = 页面

query-query-workspace-content = 内容

query-query-workspace-query-results-grouped-by-page = 按页面分组的查询结果

query-query-display-move-up = 上移 { $value1 }

query-query-display-move-down = 下移 { $value1 }

query-query-display-remove = 移除 { $value1 }

query-query-display-display-pages = 显示页面

query-query-display-display = 显示

query-query-display-page-display = 页面显示方式

query-query-display-block-display = 块显示方式

query-query-display-view = 视图

query-query-display-query-view = 查询视图

query-query-display-group-by = 分组依据

query-query-display-no-grouping = 不分组

query-query-display-none = 无

query-query-display-sort = 排序

query-query-display-asc = 升序

query-query-display-desc = 降序

query-query-display-columns = 列

query-query-display-summarize = 汇总

query-query-display-count = + 计数

query-query-display-sample = 抽样

query-query-display-sample-size = 抽样大小

query-query-display-no-limit = 无限制

query-query-display-a-sample-is-a-whole-number-up-to-4294967295 = 抽样大小必须是整数，最大为 4294967295。

query-query-display-no-results-sample-0 = 无结果（抽样为 0）

query-query-display-done = 完成

query-query-sheet-query-filter = 查询筛选条件

query-query-sheet-try = 试试：

query-query-sheet-query-conditions = 查询条件

query-query-sheet-conditions = 条件，{ $value1 }

query-query-sheet-switch-between-all-of-any-of = 切换全部满足 / 满足任意

query-query-sheet-group-actions = 分组操作

query-query-sheet-remove-none-of = 移除全部不满足

query-query-sheet-ungroup = 取消分组

query-query-sheet-remove = 移除

query-query-sheet-edit-in-the-query-text-below = 在下方查询文本中编辑

query-query-sheet-edit-this-part-in-the-query-text-below = 在下方查询文本中编辑此部分

query-query-sheet-edit-in-the-query-text = 在查询文本中编辑：{ $value1 }

query-query-sheet-remove-condition = 移除条件

query-query-sheet-row-actions = 行操作

query-query-sheet-this-condition-was-not-understood = 无法理解此条件。

query-query-sheet-did-you-mean = 你是否想用

query-query-sheet-edit-as-text = 以文本形式编辑

query-query-sheet-condition-value = 条件值

query-query-result-parts-edit-properties-of = 编辑 { $value1 } 的属性

query-query-result-parts-edit-page-properties = 编辑页面属性

query-query-result-parts-page-results = 页面结果

query-query-result-parts-page-results-grouped = 已分组的页面结果

query-query-result-parts-no-value = 无值

query-query-result-parts-clear-grouping = 清除分组

query-query-result-parts-a-row-with-several-tags-appears-in-every-matching-group = 一行若有多个标签，会出现在每个匹配的分组中，因此分组计数之和可能大于结果数。

query-query-result-parts-this-query-has-no-conditions-so-it-shows-nothing-as = 此查询没有条件，因此不显示结果，与 Logseq 一致。

query-query-result-parts-on-its-own-it-matches-nothing = 单独使用时不匹配任何内容。

query-query-live-preview-live-results = 实时结果

query-query-live-preview-running = 正在运行…

query-query-live-preview-page = 页面

query-query-live-preview-nothing-matches-these-conditions = 没有内容匹配这些条件。

query-query-live-preview-more-pages-match-than-are-shown = 还有更多匹配的页面未显示。

query-query-live-preview-more-blocks-match-than-are-shown = 还有更多匹配的块未显示。

query-query-builder-query-text = 查询文本

query-query-builder-raw-query-dsl = 原始查询 DSL

query-query-builder-query-text-tql = 查询文本（TQL）

query-query-builder-query-expression = 查询表达式

query-query-builder-checking = 正在检查…

query-query-builder-save-query-text = 保存查询文本

query-query-builder-in-a-disabled-condition-the-query-still-runs = { " " }（位于已禁用的条件中，查询仍会运行）

query-query-builder-show-me = 显示给我

query-query-builder-properties-in-this-graph = 此图谱中的属性：{ " " }

query-filter-condition = 条件

query-filter-page-tag-reference = 页面 / 标签引用

query-filter-task-marker = 任务标记

query-filter-priority = 优先级

query-filter-property = 属性

query-filter-scheduled = 计划日期

query-filter-deadline = 截止日期

query-filter-in-a-journal-page = 位于日记页面

query-filter-between-dates = 日期范围

query-filter-full-text-search = 全文搜索

query-filter-page = 页面

query-filter-page-property = 页面属性

query-filter-page-tags = 页面标签

query-filter-references = 引用

query-filter-does-not-reference = 不引用

query-filter-is-any-of = 属于任意一项

query-filter-is-none-of = 不属于任何一项

query-filter-is = 等于

query-filter-is-not = 不等于

query-filter-is-set = 已设置

query-filter-is-not-set = 未设置

query-filter-is-a-journal-page = 是日记页面

query-filter-is-not-a-journal-page = 不是日记页面

query-filter-between = 介于

query-filter-not-between = 不介于

query-filter-contains = 包含

query-filter-does-not-contain = 不包含

query-operator-in-namespace = 位于命名空间

query-filter-not-in-namespace = 不在命名空间

query-filter-matches = 匹配

query-filter-does-not-match = 不匹配

# Helper-return and component-prop interface text (GH #107).
query-prose-open-tasks = 未完成任务
query-prose-any-task = 任意任务
query-prose-a-b-or-c = A、B 或 C
query-prose-move-group = 移动分组
query-prose-select-group = 选择分组
query-prose-group-actions = 分组操作
query-prose-group-enabled = 启用分组
query-prose-move-condition = 移动条件
query-prose-select-condition = 选择条件
query-prose-condition-enabled = 启用条件
query-prose-row-actions = 行操作
query-prose-condition-operator = 条件运算符
query-prose-page-property = 页面属性
query-prose-property = 属性
query-prose-not = 不匹配
query-prose-matches = 匹配
query-prose-reading-this-graph-s-properties = 正在读取此图谱的属性…
query-prose-type-to-search-page-names-and-block-text = 输入内容以搜索页面名称和块文本。
query-prose-page = 页面
query-prose-block = 块
query-prose-what-this-query-selects = 此查询选择的内容
query-prose-group-the-selected-conditions = 将所选条件分组
query-prose-disabled-by-group = 已随分组停用
query-prose-disabled = 已停用
query-prose-journal-date-field = 日记日期
query-prose-scheduled = 计划时间
query-prose-deadline = 截止时间
query-prose-any-date = 任意日期
query-prose-count = 计数
query-prose-sum = 总和
query-prose-avg = 平均值

# Complete query clauses; variables containing chip segments may be reordered.
query-prose-advanced = ⟨高级条件⟩
query-prose-today = 今天
query-prose-yesterday = 昨天
query-prose-tomorrow = 明天
query-prose-journal-date = 日记日期
query-prose-not-group = 非
query-prose-everything = 全部内容
query-prose-nothing = 无内容
query-prose-on-journal = 位于日记页面
query-prose-any-status = 任意状态
query-prose-no-markers = 无任务标记
query-prose-no-levels = 无优先级
query-prose-field-value = { $field }：{ $value }
query-prose-on-page = 页面：{ $value }
query-prose-in-namespace = 命名空间：{ $value }
query-prose-page-property-clause = 页面的{ $inner }
query-prose-page-tags = 页面标签：{ $value }
query-prose-property-any = { $field }：任意值
query-prose-task = 任务：{ $value }
query-prose-priority = 优先级：{ $value }
query-prose-content = 文本：“{ $value }”
query-prose-search = 搜索：{ $value }
query-prose-date-range = { $low }至{ $high }
query-prose-off = { $inner }（已停用）
query-prose-negation = 非{ $inner }
query-prose-parentheses = （{ $inner }）
query-prose-sentence = 符合{ $predicate }的{ $subject }
query-prose-attr-content = 文本
query-prose-attr-task = 任务
query-prose-attr-priority = 优先级
query-prose-attr-scheduled = 计划时间
query-prose-attr-deadline = 截止时间
query-prose-attr-created-at = 创建时间
query-prose-attr-last-modified-at = 最后修改时间
query-prose-attr-name = 名称
query-prose-attr-journal = 日记
query-prose-attr-day = 日期
query-prose-attr-namespace = 命名空间
query-prose-attr-used-as-tag = 用作标签
query-prose-attr-key = 键
query-prose-attr-value = 值
query-prose-attr-atom-count = 值
query-prose-property-operator =
    { $op ->
       *[is] { $cardinality ->
                [list] 包含此值
               *[scalar] 是
            }
        [is_not] { $cardinality ->
                [list] 不包含此值
               *[scalar] 不是
            }
        [gt] 大于
        [ge] 不小于
        [lt] 小于
        [le] 不大于
        [between] 介于
        [before] 早于
        [on_or_before] 不晚于
        [after] 晚于
        [on_or_after] 不早于
        [contains] 包含
        [does_not_contain] 不包含
        [starts_with] 开头是
        [ends_with] 结尾是
        [references] 引用
        [does_not_reference] 不引用
        [is_checked] 已勾选
        [is_unchecked] 未勾选
        [is_set] 已设置
        [is_not_set] 未设置
        [is_blank] 为空
        [has_other_value] 有其他值，异于
    }
query-prose-property-test =
    { $op ->
       *[is] { $field }是{ $value }
        [is_not] { $field }不是{ $value }
        [gt] { $field }大于{ $value }
        [ge] { $field }不小于{ $value }
        [lt] { $field }小于{ $value }
        [le] { $field }不大于{ $value }
        [between] { $field }介于{ $value }
        [before] { $field }早于{ $value }
        [on_or_before] { $field }不晚于{ $value }
        [after] { $field }晚于{ $value }
        [on_or_after] { $field }不早于{ $value }
        [contains] { $field }包含{ $value }
        [does_not_contain] { $field }不包含{ $value }
        [starts_with] { $field }开头是{ $value }
        [ends_with] { $field }结尾是{ $value }
        [references] { $field }引用{ $value }
        [does_not_reference] { $field }不引用{ $value }
        [is_checked] { $field }已勾选
        [is_unchecked] { $field }未勾选
        [is_set] { $field }已设置
        [is_not_set] { $field }未设置
        [is_blank] { $field }为空
        [has_other_value] { $field }有其他值，异于{ $value }
    }
query-prose-attribute-test =
    { $op ->
       *[eq] { $field }是{ $value }
        [not_eq] { $field }不是{ $value }
        [lt] { $field }<{ $value }
        [le] { $field }≤{ $value }
        [gt] { $field }>{ $value }
        [ge] { $field }≥{ $value }
        [between] { $field }介于{ $value }
        [in] { $field }属于{ $value }
        [not_in] { $field }不属于{ $value }
        [like] { $field }包含{ $value }
        [starts_with] { $field }开头是{ $value }
        [match] { $field }匹配{ $value }
        [regex] { $field }匹配正则表达式{ $value }
        [is_set] { $field }已设置
        [is_not_set] { $field }未设置
        [is_blank] { $field }为空
    }
query-prose-boolean =
    { $value ->
        [true] 是
       *[false] 否
    }
query-prose-group =
    { $kind ->
       *[and] 且
        [or] 或
    }
query-prose-subject =
    { $family ->
        [both] 页面和块
        [page] 页面
       *[block] 块
    }
query-prose-all =
    { $family ->
        [both] 所有页面和块
        [page] 所有页面
       *[block] 所有块
    }
query-prose-date-bound =
    { $op ->
       *[ge] 从{ $when }起
        [le] 截至{ $when }
    }
query-prose-relative-days = { $count } 天{ $direction ->
        [ago] 前
       *[ahead] 后
    }
query-prose-relative-weeks = { $count } 周{ $direction ->
        [ago] 前
       *[ahead] 后
    }
query-prose-relative-months = { $count } 个月{ $direction ->
        [ago] 前
       *[ahead] 后
    }
query-prose-relative-years = { $count } 年{ $direction ->
        [ago] 前
       *[ahead] 后
    }
query-prose-list = { $left }{ $position ->
        [middle] 、
       *[last] { $connector ->
            [and] 且
           *[or] 或
        }
    }{ $right }
query-prose-relation = { $quant ->
        [every] 所有
        [none] 无
       *[any] 任意
    }{ $relation ->
        [page] 页面
        [refs] 引用
        [direct_refs] 直接引用
        [tags] 标签
        [props] 属性
       *[children] 子块
        [parent] 父块
        [descendants] 后代块
        [ancestors] 祖先块
        [blocks] 块
    }：{ $inner }

query-prose-not-symbol = 非 { $symbol }
query-prose-invalid-regex = 正则表达式无效：{ $error }
query-prose-regex-summary = 使用区分大小写的正则表达式 /{ $pattern }/ 匹配页面名称或块文本。
query-prose-search-summary = 匹配{ $group }的页面名称或块文本。
query-prose-search-groups-summary = 匹配以下条件的页面名称或块文本：{ $groups }。
query-prose-search-and = { $left }且{ $right }
query-prose-search-or = { $left }；或{ $right }
query-prose-search-term = { $kind ->
        [exclude-exact] 不包含完整短语“{ $value }”
        [exclude] 不包含“{ $value }”
        [exact] 包含完整短语“{ $value }”
       *[contains] 包含“{ $value }”
    }
query-prose-aggregate-field = { $field }的{ $verb }

query-prose-advanced-clause = { $advanced } { $clause }
query-prose-aggregate-unavailable = 不可用（{ $reason ->
        [non_finite] 非有限数值
        [division_by_zero] 除以零
        [empty_group] 空分组
       *[non_numeric] 非数值
    }）
query-prose-formula-statistics-notice = 暂不支持按公式精确统计，当前显示总体统计。
`;export{e as default};
