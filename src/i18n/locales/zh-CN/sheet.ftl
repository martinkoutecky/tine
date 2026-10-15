sheet-can-t-convert-to-grid-children-would-collide-with-grid-rows = 无法转换为网格：子块会与网格行冲突。

sheet-can-t-convert-this-parser-does-not-round-trip-inside-table-cells = 无法转换：此解析器无法在表格单元格内完整保留 \|。

sheet-couldn-t-cut-cells-clipboard-write-failed = 无法剪切单元格：写入剪贴板失败。

sheet-pasted-over-existing-cells = 已粘贴并覆盖现有单元格。

sheet-undo = 撤销

sheet-pasted-outline-is-too-deep = 粘贴的大纲层级过深

sheet-couldn-t-copy-cells-clipboard-write-failed = 无法复制单元格：写入剪贴板失败。

sheet-resize-column = 调整 { $label } 列宽

sheet-drag-to-resize-double-click-to-reset = 拖动调整大小，双击重置

sheet-grid-requires-markdown = 仅 Markdown 网格可转换为竖线表格。

sheet-grid-read-only = 无法将只读网格转换为竖线表格。

sheet-grid-title-required = 无法转换：网格块需要在 tine.* 属性上方有一行标题。

sheet-grid-too-large = 超过 30 列、200 行的网格无法转换为竖线表格。

sheet-grid-empty = 无法将空网格转换为竖线表格。

sheet-grid-row-missing = 网格缺少一行，无法转换。

sheet-grid-row-properties = 无法转换：行的隐藏属性会丢失。

sheet-grid-row-content = 无法转换：行块内容必须为空。

sheet-grid-row-empty = 无法转换：空行无法用竖线表格表示。

sheet-grid-cell-missing = 网格缺少一个单元格，无法转换。

sheet-grid-cell-properties = 无法转换：单元格的隐藏属性会丢失。

sheet-grid-cell-children = 无法转换：有子块的单元格无法用竖线表格表示。

sheet-grid-cell-multiline = 无法转换：单元格必须为单行。

sheet-grid-cell-pipe = 无法转换：此解析器无法在表格单元格内完整保留 \|。

sheet-load-more = 再加载 { NUMBER($count, useGrouping: 0) } { $kind ->
    [cards] 张卡片
   *[other] 行
    }（{ NUMBER($shown, useGrouping: 0) } / { NUMBER($total, useGrouping: 0) }）

sheet-rename-field = { $hasReason ->
    [1] 重命名字段…（{ $reason }）
   *[other] 重命名字段…
    }

sheet-cell-refused = 无法将该单元格{ $hasValue ->
    [1] { " " }改为“{ $value }”
   *[other] { "" }
    }：页面只读、行已不存在，或值不符合列要求。

sheet-type-text = 文本

sheet-type-number = 数字

sheet-type-date = 日期

sheet-type-datetime = 日期时间

sheet-type-checkbox = 复选框

sheet-type-list = 列表

sheet-type-ref = 引用

sheet-sheet-table-this-order-cannot-be-saved-while-a-computed-column-is = 显示计算列时，无法保存此顺序。

sheet-sheet-table-some-direct-rows-are-not-loaded-no-fields-were-renamed = 部分直接子行未加载，未重命名任何字段。

sheet-sheet-table-rename-field-formula-columns-cannot-be-renamed-here = 重命名字段…（公式列无法在此重命名）

sheet-sheet-table-rename-field-only-children-backed-tables-can-rename-fields = 重命名字段…（仅以子块为来源的表格可重命名字段）

sheet-sheet-table-edit-formula = 编辑公式…

sheet-sheet-table-remove-formula = 移除公式

sheet-sheet-table-add-formula = 添加公式…

sheet-sheet-table-declare-field-text = 声明字段（文本）

sheet-sheet-table-type = 类型 →

sheet-sheet-table-remove-from-schema = 从结构定义中移除

sheet-sheet-table-remove-column = 移除列

sheet-sheet-table-empty-table = 空表格

sheet-sheet-table-add-row = 添加行

sheet-sheet-table-block = 块

sheet-sheet-table-clear-table-only-sort = 清除仅用于此表格的排序

sheet-sheet-table-filter-disabled = 筛选已禁用

sheet-sheet-table-rename-field-3 = 重命名字段 { $value1 }

sheet-sheet-table-add-column = 添加列

sheet-sheet-table-property = 属性

sheet-sheet-table-cell-menu = 单元格菜单

sheet-sheet-table-clear = 清除

sheet-sheet-board-moved-out-of-this-query-s-results = 已移出此查询结果

sheet-sheet-board-all-results = 全部结果

sheet-sheet-board-invalid-tag-name = 标签名称无效

sheet-sheet-board-group-by = 分组依据

sheet-sheet-board-no-grouping = 不分组

sheet-sheet-board-empty-board = 空看板

sheet-sheet-board-add-tag-column = 添加标签列

sheet-sheet-board-new-tag = 新标签

sheet-sheet-board-new-tag-2 = new-tag

sheet-sheet-board-card-menu = 卡片菜单

# Helper-return and component-prop interface text (GH #107).
sheet-prose-state = 状态
sheet-prose-priority = 优先级
sheet-prose-scheduled = 计划时间
sheet-prose-deadline = 截止时间
sheet-prose-tags = 标签
sheet-prose-page = 页面

sheet-prose-table-only-sort = 仅对表格排序：{ $field }
sheet-prose-title = 标题
sheet-board-empty-column = （无）
sheet-aggregate-none = 无
sheet-aggregate-name = { $fn ->
    [sum] 求和
    [average] 平均值
    [median] 中位数
    [min] 最小值
    [max] 最大值
    [range] 极差
    [stddev] 标准差
    [earliest] 最早
    [latest] 最晚
    [empty] 空值数
    [filled] 非空值数
    [unique] 唯一值数
    [checked] 已勾选数
    [unchecked] 未勾选数
   *[count] 计数
    }
