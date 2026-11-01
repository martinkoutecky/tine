## Margin dialogue: comments on a passage (Ctrl/Cmd+R)
comment-on-whole-block = on the whole block
comment-quote-changed = quoted text changed
# $author (string): the value of the block's author:: property.
comment-written-by = Written by { $author }
comment-written-by-you = Written by you
comment-author-you = you
# $count (number): how many comments the block has in the margin.
comment-margin-count = { NUMBER($count, useGrouping: 0) } { $count ->
        [one] comment
       *[other] comments
    } in the margin
comment-margin-label = Comments
comment-toolbar = Comment
comment-toolbar-shortcut = Comment (Ctrl/Cmd+R)
comment-reply = Reply
comment-reply-placeholder = Reply…
comment-delete = Delete comment and replies
comment-select-first = Select text in a block first, then choose Comment on selection (or press Ctrl+R while editing)
comment-block-changed = The block changed after you selected text; select it again to comment
comment-block-cannot-take = This block cannot take a comment here
comment-stale-write-what = The comment

## Settings → Editing: the name your comments carry
comment-name-label = Your name
comment-name-hint = Comments and replies you write in the margin are saved with this name as <code>author::</code>, so a graph shared with other people shows who wrote what. Leave it empty to write no author; your own comments then show as "you". Stored on this device, not in the graph.
comment-name-placeholder = Your name

## The one-time "Your name" question before the first comment or reply in a git graph (tine#841)
comment-name-prompt-title = Sign your comments?
comment-name-prompt-body = This graph is in a git repository, so other people may read your comments. With a name, each comment and reply you write is saved with it, and they can tell who wrote what. Without one, your comments show as "you" to everyone.
comment-name-prompt-scope = Your name is a Tine setting on this device, used in all your graphs, not a setting of this graph. Change it any time in Settings > Editor > Your name. If you skip, Tine does not ask again.
comment-name-prompt-dismiss = Skip
comment-name-prompt-save = Use this name
comment-name-prompt-graph-changed = The graph changed while Tine asked for your name, so the comment was not written.
comment-margin-draft-pending = Send or discard the unsent reply before hiding the comments
