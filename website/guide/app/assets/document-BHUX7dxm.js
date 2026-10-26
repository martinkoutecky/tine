const e=`document-review-unsaved = 检查未保存的内容

document-create-unreadable = 无法创建“{ $name }”：{ $hasOwner ->
    [1] { $owner }
   *[other] 某个页面文件
    }无法读取，它可能已是此页面。请修复或移动该文件；你的编辑仍保留在编辑器中。

document-read-refused = 无法从磁盘读取“{ $name }”，你的编辑仍保留在编辑器中。

document-draft-refused = 无法为“{ $name }”保留崩溃恢复副本，你的编辑仍保留在编辑器中。

document-save-refused = 无法保存“{ $name }”：{ $detail }

document-save-twin = 无法保存“{ $name }”：{ $existing } 已是此页面。你的编辑仍保留在编辑器中。

document-save-undecodable = 无法保存“{ $name }”：其文件不是可读文本。你的编辑仍保留在编辑器中。

document-save-retrying = 暂时无法保存“{ $name }”{ $step }，Tine 会继续重试，并保留崩溃恢复副本。

document-operation-unrecorded = Tine 无法确认的“{ $name }”重命名或删除操作未发生：无法记录该操作。内容未更改。

document-draft-unwritten = 无法写入“{ $name }”的崩溃恢复副本。

document-load-refused = “{ $page }”已从{ $hasHolder ->
    [1] { $holder }
   *[other] 尚未保存的新页面
    }打开，其中有未保存的内容，因此未加载{ $hasRequested ->
    [1] { $requested }
   *[other] 该页面
    }。请先完成或保存该编辑。{ $hasAfter ->
    [1] { " " }{ $after }
   *[other] { "" }
    }

document-move-incomplete = { $completed ->
    [0] 未移动：无法先保存“{ $pages }”。
   *[other] 仅完成部分移动：“{ $pages }”中的块未移动。
    }

document-external-updates = { NUMBER($pages, useGrouping: 0) } 个页面被外部更新{ $conflicts ->
    [0] { "" }
    [one] { " " }· { NUMBER($conflicts, useGrouping: 0) } 处冲突待检查
   *[other] { " " }· { NUMBER($conflicts, useGrouping: 0) } 处冲突待检查
    }

document-wiring-tine-can-t-save-this-graph = Tine 无法保存此图谱：{ $value1 }

document-wiring-saved-the-page-but-could-not-refresh-its-title = 页面已保存，但无法刷新标题：{ $value1 }

document-wiring-saved-the-title-but-its-page-identity-could-not-be = 标题已保存，但无法安全采用其页面标识。请通过文件路径重新打开此页面。

document-wiring-resolve-the-conflict-on-first = 请先解决“{ $value1 }”的冲突。

document-wiring-changed-meanwhile-choose-again = “{ $value1 }”期间发生了更改，请重新选择。

document-wiring-couldn-t-overwrite = 无法覆盖“{ $value1 }”。

document-wiring-couldn-t-read-the-page-is-an-alias-of-its = 无法读取 { $value1 }，“{ $value2 }”是它的别名。文本仍保留在这里，尚未保存。

document-wiring-changed-since-was-added-to-it-copy-the-rest-of = 将“{ $value2 }”添加到“{ $value1 }”后，后者又发生了更改。请手动复制“{ $value3 }”的剩余内容。

document-wiring-moved-into-its-alias-owner = 已将“{ $value1 }”移入其别名所属页面“{ $value2 }”。
`;export{e as default};
