const n=`conflict-bar-review = 检查

conflict-bar-use-disk-version = 使用磁盘版本

conflict-bar-keep-mine-overwrite = 保留我的版本（覆盖）

conflict-overview-unsaved-drafts = 未保存的草稿

conflict-overview-sync-conflict-copies = 同步冲突副本

conflict-overview-version-control-merge-markers = 版本控制合并标记

conflict-overview-duplicate-journal-days = 同一天的重复日记

conflict-overview-conflicts = 冲突

conflict-overview-no-conflicts-pages-that-need-a-decision-appear = 没有冲突。需要你决定如何处理的页面会出现在这里。

conflict-overview-each-page-is-resolved-on-the-page-itself = 请在各页面内逐块解决冲突。按住 Shift 点击可在右侧边栏打开页面，同时保留此列表。

conflict-overview-open-the-page-to-resolve-it-shift-click = 打开页面以解决冲突（Shift 点击：右侧边栏）

conflict-overview-discard-copy = 丢弃副本

conflict-overview-conflict-copy = 冲突副本

conflict-resolution-previous-conflict = 上一个冲突

conflict-resolution-next-conflict = 下一个冲突

conflict-resolution-this-file-still-contains-merge-markers-so-tine = 此文件仍含有合并标记，Tine 无法保存：重写文件会改变标记的缩进，导致一方内容在无提示的情况下丢失。请在下方选择并应用，将其恢复为普通页面。

conflict-resolution-choosing-below-folds-the-other-file-into-this = 在下方选择后，会将另一文件合并到当天日记，并移入可恢复的回收站。也可直接操作文件：

conflict-resolution-reading-both-versions = 正在读取两个版本…

conflict-resolution-these-two-files-can-t-be-folded-together = 这两个文件无法合并：一个是 Markdown，另一个是 Org。请使用上方的文件操作。

conflict-resolution-couldn-t-read-this-conflict = 无法读取此冲突。（{ $error }）

conflict-resolution-conflict-unavailable = 无法读取此冲突。

conflict-resolution-the-two-versions-are-identical-nothing-to-decide = 两个版本完全相同，无需选择。

conflict-resolution-the-copy-is-safe-to-discard-from-the = { " " }可在冲突概览中安全丢弃副本。

conflict-resolution-the-other-file-is-safe-to-trash-above = { " " }可在上方安全地将另一文件移入回收站。

conflict-resolution-use-disk-version-above-loses-nothing = { " " }上方的“使用磁盘版本”不会丢失任何内容。

conflict-resolution-nothing-was-pre-selected-no-common-version-is = 未预选任何内容。由于没有已知的共同版本，两方内容都被保留。

conflict-resolution-re-applies-tine-s-own-suggestions-a-merge = 重新应用 Tine 的建议。合并工具提供的文本会保留你当前的选择。

conflict-resolution-apply-all-suggested = 应用全部建议

conflict-resolution-keep-both-everywhere = 全部保留两方内容

conflict-resolution-show-unchanged = 显示未更改的内容

conflict-resolution-the-page-s-own-properties-differ-keep = 页面本身的属性不同。保留

conflict-resolution-both-merge = 两方（合并）

conflict-resolution-applying-writes-the-merged-page-without-any-markers = 应用后将写入不含标记的合并页面；原文件会移入可恢复的回收站。

conflict-resolution-applying-writes-the-merged-page-only-if-the = 仅当文件仍是此处显示的版本时才会写入合并页面；若有更新的更改，将刷新此对比。

conflict-resolution-the-other-file-moves-to-the-recoverable-trash = 应用后，另一文件会移入可恢复的回收站，当天日记只保留一个文件。

conflict-resolution-the-copy-moves-to-the-recoverable-trash-once = 应用后，副本会移入可恢复的回收站。

conflict-resolution-applying = 正在应用…

conflict-resolution-apply-resolution = 应用解决方案

diff-rows-earlier-lines-are-the-same-on-both-sides = 前面的行在两方中相同

diff-rows-merged = 已合并

diff-rows-unchanged = 未更改

diff-rows-pre-selected-from-the-last-version-both-sides = 根据两方最后一致的版本预选

diff-rows-suggested = 建议

diff-rows-hide-the-full-bodies = 隐藏完整正文

diff-rows-show-all-lines = 显示全部 { $line_count } 行

diff-rows-merged-tool = 已合并（工具）

journal-conflict-file-row-show-this-file-s-contents = 显示此文件的内容

journal-conflict-file-row-canonical = { " " }· 主文件

journal-conflict-file-row-open-this-exact-file-editable = 打开此文件（可编辑）

journal-conflict-file-row-open = 打开

journal-conflict-file-row-append-this-file-s-blocks-to-the-canonical = 将此文件的块追加到当天日记的主文件，然后移入回收站

journal-conflict-file-row-merge = 合并

journal-conflict-file-row-move-this-file-to-a-uniquely-named-page = 将此文件移到名称唯一的页面

journal-conflict-file-row-rename = 重命名…

journal-conflict-file-row-trash = 移入回收站

journal-conflict-file-row-couldn-t-read-this-file = 无法读取此文件：{ $error }

journal-conflict-file-row-new-page-name = 新页面名称

journal-conflict-file-row-save = 保存

journal-conflict-file-row-cancel = 取消

journal-conflict-file-row-empty-file = （空文件）

journal-filename-panel-journal-files-named-by-title = 以标题命名的日记文件

journal-filename-panel-these-journal-files-aren-t-named-after-their = 这些日记文件未按日期命名，Tine 无法将其放入日记列表，对应日期看起来是空白的。重命名可解决此问题，但会更改你的文件，因此 Tine 不会自动操作。版本控制和同步工具会将其视为重命名。

journal-filename-panel-renaming = 正在重命名…

journal-filename-panel-rename-to-date-names = 重命名为日期

live-conflicts-your-unsaved-edits = 你未保存的编辑

live-conflicts-the-file-on-disk-now = 当前磁盘上的文件

mock-conflicts-this-device = 本设备

conflict-bar-live-notice = <strong>你编辑“{ $name }”时，磁盘上的文件发生了更改。</strong>你的编辑仍保留，请在页面上与磁盘版本对比。

conflict-bar-unsaved-notice = { $deleted ->
    [1] <strong>磁盘上的“{ $name }”已删除</strong>。未保存的更改尚未写入。
   *[other] <strong>磁盘上的“{ $name }”发生了更改</strong>。未保存的更改尚未写入。
    }

conflict-overview-orphan-source = 同步副本 · { $tag } · 原页面已不存在

conflict-overview-discard-confirmation = 丢弃冲突副本“{ $name }”吗？{ "\\u000A\\u000A" }副本将移入 logseq/.tine-trash（可恢复）。当前的“{ $page }”保持原样。

conflict-resolution-trash-confirmation = 将日记文件“{ $name }”移入回收站吗？{ "\\u000A\\u000A" }它与另一个文件属于同一天，将移入 logseq/.tine-trash（可恢复）。

conflict-resolution-count = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } 处冲突
   *[other] { NUMBER($count, useGrouping: 0) } 处冲突
    }

conflict-resolution-base-suggestions = { $base }（用于生成建议）

conflict-resolution-suggested-summary = 已根据两方最后一致的版本预选 { $suggested } / { $total } 项，请检查并确认。

conflict-resolution-keep-side = 保留 { $side }

conflict-resolution-review-count = 待检查：{ $count }

journal-filename-panel-confirmation = { $count ->
    [one] 将 { NUMBER($count, useGrouping: 0) } 个日记文件重命名为日期吗？{ "\\u000A\\u000A" }会先创建快照，原文件名保留在“备份与恢复”中。不会覆盖任何文件：日期名称已被占用的文件会保持原样。
   *[other] 将 { NUMBER($count, useGrouping: 0) } 个日记文件重命名为日期吗？{ "\\u000A\\u000A" }会先创建快照，原文件名保留在“备份与恢复”中。不会覆盖任何文件：日期名称已被占用的文件会保持原样。
    }

# Helper-return and component-prop interface text (GH #107).
conflicts-prose-unsaved-draft = 未保存的草稿
conflicts-prose-mine = 我的版本
conflicts-prose-theirs = 对方的版本
conflicts-prose-mine-short = 我的
conflicts-prose-theirs-short = 对方
conflicts-prose-unresolved-merge-from-your-version-control-tool = 版本控制工具留下的未解决合并
conflicts-prose-your-edits-and-a-newer-version-on-disk = 你的编辑与磁盘上的较新版本
conflicts-prose-this-day-has-more-than-one-file = 这一天有多个文件
conflicts-prose-two-versions-of-this-page-arrived = 收到了此页面的两个版本
conflicts-prose-proposed-by-your-merge-tool-s-suggested-resolution-still-applied = 合并工具建议的解决方案，仍需你确认后才会应用。
conflicts-prose-both-edits-combined-offered-because-they-touch-different-parts-of = 已合并双方的编辑，因为它们修改了正文的不同部分
conflicts-prose-both = 双方
conflicts-prose-merged = 已合并
conflicts-prose-keep = 保留
conflicts-prose-drop = 舍弃
conflicts-prose-skip = 跳过
conflicts-prose-pull-in = 引入

conflicts-prose-day-files = 这一天有 { $count } 个文件
conflicts-prose-sync-copy = 同步副本 · { $side }
conflicts-prose-merge-sides = 合并标记 · { $mine } 与 { $theirs }
conflicts-prose-block-count = { $count } 个块

conflicts-prose-copy-fallback = 冲突副本
conflicts-prose-local-fallback = 本地
conflicts-prose-incoming-fallback = 合入
conflicts-prose-copy-month = { $month }月
conflicts-prose-dated-copy = 同步副本 · { $year ->
        [current] { "" }
       *[other] { $year }年
    }{ $month }{ $day }日

conflicts-prose-local-side = 本地一方
conflicts-prose-merged-side = 合入一方
conflicts-prose-conflict-copy = 冲突副本
`;export{n as default};
