## 自上次查看以来的更改（git 图谱；ADR 0079）
# $count (number): changed or removed blocks on this page since you last left it.
seen-header-count = 自上次查看以来有 { NUMBER($count, useGrouping: 0) } { $count ->
        [one] 处更改
       *[other] 处更改
    }
seen-mark-page = 标记页面为已看
seen-removed-block = 此处有一个块被删除
seen-note-identity-unknown = 此图谱未设置 git 身份，因此只有 Tine 以占位名称提交的内容才算作你的。
seen-note-unattributed = 自你上次查看以来此页面的历史很长，因此其中所有更改（包括你自己的）都会计入。
# $name (string): the page's name.
seen-page-marked = { $name } 已标记为已看
seen-graph-marked = 整个图谱已标记为已看
seen-state-forgotten = 已忘记此图谱的已看状态
# $error (string): why marking failed.
seen-mark-failed = 无法标记为已看：{ $error }
# $error (string): why forgetting failed.
seen-forget-failed = 无法忘记已看状态：{ $error }
seen-settings-label = 自上次查看以来的更改
seen-settings-hint = 在 git 图谱中，Tine 会标出你上次离开页面后其他人或工具提交的块。该记录只保存在本设备上，不会写入图谱。未提交的编辑不可见，你自己的提交永远不会计入。
seen-settings-mark-graph = 标记整个图谱为已看
seen-settings-forget = 忘记已看状态
