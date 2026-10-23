## Margin dialogue: comments on a passage (Ctrl/Cmd+R)
comment-on-whole-block = 针对整个块
comment-quote-changed = 引用的文字已更改
# $author (string): the value of the block's author:: property.
comment-written-by = 作者：{ $author }
comment-written-by-you = 作者：你
comment-author-you = 你
# $count (number): how many comments the block has in the margin.
comment-margin-count = 边栏中有 { NUMBER($count, useGrouping: 0) } { $count ->
        [one] 条评论
       *[other] 条评论
    }
comment-margin-label = 评论
comment-toolbar = 评论
comment-toolbar-shortcut = 评论 (Ctrl/Cmd+R)
comment-reply = 回复
comment-reply-placeholder = 回复…
comment-delete = 删除评论及其回复
comment-select-first = 请先在块中选中文字，再选择“评论所选内容”（或在编辑时按 Ctrl+R）
comment-block-changed = 选中文字后该块已发生更改，请重新选中后再评论
comment-block-cannot-take = 此处的块无法添加评论
comment-stale-write-what = 该评论

## Settings → Editing: the name your comments carry
comment-name-label = 你的名字
comment-name-hint = 你在页边写的批注和回复会以此名字保存为 <code>author::</code>，这样与他人共享的图谱就能看出谁写了什么。留空则不写作者；此时你自己的批注显示为“你”。保存在本设备上，不写入图谱。
comment-name-placeholder = 你的名字
