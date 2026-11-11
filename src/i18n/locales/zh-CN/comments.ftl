## Margin dialogue: comments on a passage (Ctrl/Cmd+R)
comment-on-whole-block = 针对整个块
comment-quote-changed = 引用的文字已更改
# $author (string): the value of the block's tine.author:: property.
comment-written-by = 作者：{ $author }
comment-written-by-you = 作者：你
comment-author-you = 你
# $count (number): how many comments the block has in the margin.
comment-margin-count = 边栏中有 { NUMBER($count, useGrouping: 0) } { $count ->
        [one] 条评论
       *[other] 条评论
    }
comment-quote-ambiguous = 引用文字位置不明确
comment-margin-label = 评论
comment-toolbar = 评论
comment-toolbar-shortcut = 评论 (Ctrl/Cmd+R)
comment-reply = 回复
comment-send = 发送
comment-new-thread = 新建讨论串
comment-resolve = 标为已解决
comment-reopen = 重新打开
comment-resolved-count = { NUMBER($count, useGrouping: 0) } 个已解决
comment-below = 下方 { NUMBER($count, useGrouping: 0) } 个
comment-below-expand = 展开以显示下方 { NUMBER($count, useGrouping: 0) } 个评论讨论串
comment-below-changed = 下方 { NUMBER($count, useGrouping: 0) } 个，其中 { NUMBER($changed, useGrouping: 0) } 个有更改
comment-below-changed-expand = 展开以显示下方 { NUMBER($count, useGrouping: 0) } 个评论讨论串，其中 { NUMBER($changed, useGrouping: 0) } 个自上次查看以来有更改
comment-show = 显示评论
comment-hide = 隐藏评论
# $page (string): the page the review is limited to.
comment-review-title-page = “{ $page }”上的未解决评论
comment-review-title-graph = 整个图谱中的未解决评论
comment-review-author-label = 作者
comment-review-author-placeholder = 任何作者
comment-review-open = 查看未解决的评论
comment-review-open-graph = 查看整个图谱中未解决的评论
comment-reply-placeholder = 回复…
comment-delete = 删除评论及其回复
comment-select-first = 请先在块中选中文字，再选择“评论所选内容”（或在编辑时按 Ctrl+R）
comment-block-changed = 选中文字后该块已发生更改，请重新选中后再评论
comment-block-cannot-take = 此处的块无法添加评论
comment-stale-write-what = 该评论

## Settings → Editing: the name your comments carry
comment-name-label = 你的名字
comment-name-hint = 你在页边写的批注和回复会以此名字保存为 <code>tine.author::</code>，这样与他人共享的图谱就能看出谁写了什么。留空则不写作者；此时你自己的批注显示为“你”。保存在本设备上，不写入图谱。
comment-name-placeholder = 你的名字

## 在 git 图谱中第一次批注或回复前的一次性“你的名字”提问 (tine#841)
comment-name-prompt-title = 你的批注应该如何署名？
comment-name-prompt-body = 设置名字后，你写的每条批注和回复都会带上它，别人就能看出是谁写的。不设置的话，你的批注对所有人都显示为“你”。
comment-name-prompt-scope = 你的名字是本设备上的 Tine 设置，适用于你所有的图谱，而不是此图谱的设置。随时可在 设置 > 编辑器 > 你的名字 中修改。若选择跳过，Tine 不会再询问。
comment-name-prompt-dismiss = 跳过
comment-name-prompt-save = 使用此名字
comment-name-prompt-graph-changed = 在询问你的名字时图谱已切换，因此没有写入该批注。
comment-remove-author = 移除作者
comment-margin-draft-pending = 请先发送或放弃未发送的回复，再隐藏评论
comment-open-thread = 打开评论对话
comment-unanchored-marker = { $count ->
        [one] 打开引用文字无法高亮的评论
       *[other] 打开 { NUMBER($count, useGrouping: 0) } 条引用文字无法高亮的评论
    }
comment-thread-position = 第 { $current } 处，共 { $count } 处
comment-prev-thread = 上一处
comment-next-thread = 下一处
comment-close-threads = 关闭评论
comment-review-position = 第 { $current } 条评论，共 { $count } 条
comment-collapse = 折叠
comment-expand = 展开
comment-collapse-all = 折叠此块的所有评论
comment-replies = { $count } 条回复
