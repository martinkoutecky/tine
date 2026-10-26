const e=`graph-retry = 重试

graph-restart-tine = 重启 Tine

graph-this-graph-s-assets-folder-points-outside-the = 此图谱的 assets 文件夹指向图谱外部：{ "\\u000A" }{ "\\u000A" }{ $assetsDirectory }{ "\\u000A" }{ "\\u000A" }允许 Tine 读取和写入此目录中的附件吗？授权仅保存在本设备上。

graph-allow-external-assets-directory = 允许外部附件目录吗？

graph-graph-not-opened-its-external-assets-directory = 图谱未打开：其外部附件目录未获授权（{ $assetsDirectory }）。

graph-some-pages-couldn-t-be-saved-resolve = 部分页面无法保存，请先解决冲突再切换图谱。

graph-couldn-t-prepare-the-icloud-graph = 无法准备 iCloud 图谱。（{ $error }）

graph-tineoutline-can-only-open-folders-inside-on = TineOutline 只能打开“我的 iPhone → TineOutline”或“iCloud Drive → TineOutline”中的文件夹。

graph-pdf-changes-couldn-t-be-saved-the = PDF 更改无法保存，当前图谱仍保持打开。

graph-a-rename-is-still-running-switch = 重命名仍在进行，请等待完成后再切换图谱。

graph-waiting-for-tine-to-finish-saving = 正在等待 Tine 保存“{ $saving }”，完成后将打开此图谱…

graph-recovered-unsaved-edits-on-page-s-tine = { $count ->
    [one] 已恢复 { $count } 个页面的未保存编辑，Tine 正在保存：{ $pages }。
   *[other] 已恢复 { $count } 个页面的未保存编辑，Tine 正在保存：{ $pages }。
    }

graph-page-s-still-have-edits-that-are = { $count ->
    [one] { $count } 个页面仍有尚未写入磁盘的编辑，Tine 会继续保存：{ $pages }。
   *[other] { $count } 个页面仍有尚未写入磁盘的编辑，Tine 会继续保存：{ $pages }。
    }

graph-tine-left-unreadable-crash-recovery-file-s-in-place = { $count ->
    [one] Tine 保留了 { $count } 个无法读取的崩溃恢复文件：{ $files }。
   *[other] Tine 保留了 { $count } 个无法读取的崩溃恢复文件：{ $files }。
    }

graph-crash-recovery-is-off-for-this-graph = 此图谱的崩溃恢复已关闭：{ $unavailable }。你的编辑仍会正常保存。

graph-crash-recovery-is-on-again = 崩溃恢复已重新启用。

graph-crash-recovery-is-still-off = 崩溃恢复仍未启用：{ $error }。

graph-crash-recovery-copies-from-an-older-tine-are = 旧版 Tine 的崩溃恢复副本原样保留在 { $file }。

graph-page-already-exists-merge-into = 页面“{ $to }”已存在。将“{ $from }”合并到该页面吗？

graph-couldn-t-open-the-folder-picker = 无法打开 { $platform } 文件夹选择器。（{ $error }）

graph-tine-needs-a-folder-on-the-device-s = Tine 需要设备本地存储中的文件夹，无法直接打开此提供商的文件夹。请使用提供商的同步到本地文件夹功能，再在“打开图谱”中选择该本地文件夹。

graph-grant-all-files-access-for-tine-then = 请授予 Tine“所有文件访问权限”，然后再次点击“打开”。

graph-choose-a-folder-inside-on-my-iphone = 请选择“我的 iPhone → TineOutline”或“iCloud Drive → TineOutline”中的文件夹。目前不支持其他应用的文件夹和“文件”提供商。

graph-couldn-t-prepare-the-icloud-location = 无法准备 iCloud 位置。（{ $error }）

graph-tineoutline-can-only-create-graphs-inside-on = TineOutline 只能在“我的 iPhone → TineOutline”或“iCloud Drive → TineOutline”中创建图谱。

graph-couldn-t-create-the-graph = { $graphState ->
    [open] 无法创建图谱。当前图谱仍保持打开。（{ $error }）
   *[none] 无法创建图谱。（{ $error }）
    }

graph-open-graph = 打开图谱

graph-created-the-graph-at-but-couldn-t = 已在 { $root } 创建图谱，但无法打开。（{ $detail }）

graph-created-the-graph-at-but-it = 已在 { $root } 创建图谱，但未打开。你可以再次打开此文件夹。

graph-created-the-graph-at-and-focused-its = 已在 { $root } 创建图谱，并聚焦其现有窗口。

graph-created-the-graph-at-it-is = 已在 { $root } 创建图谱。它已打开。

graph-created-and-opened-the-graph-at = 已在 { $root } 创建并打开图谱。
`;export{e as default};
