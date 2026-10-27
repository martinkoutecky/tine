graph-retry = 重试

graph-restart-tine = 重启 Tine

graph-this-graph-s-assets-folder-points-outside-the = 此图谱的 assets 文件夹指向图谱外部：{ "\u000A" }{ "\u000A" }{ $assetsDirectory }{ "\u000A" }{ "\u000A" }允许 Tine 读取和写入此目录中的附件吗？授权仅保存在本设备上。

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
root-recovery-notice = 来自 { $location } 的未保存编辑仍保留在此设备上。该图谱可能已移动或暂时不可用（例如，所在驱动器已断开连接）。仅当这些编辑属于当前图谱时才恢复。
root-recovery-earlier-location = 未知的图谱位置
# D-20261027-OF: new protection/recovery translations need native-speaker review.
owner-protection-title = 单写入者保护和命令行暂不可用
owner-protection-editable = 您可以继续编辑。另一个 Tine 进程也可能写入此图谱。Tine 每隔几秒会重试启用保护。
owner-protection-reason-namespace = Tine 无法使用此账户的私有协调文件夹。
owner-protection-reason-permissions = 此账户的协调文件夹允许其他账户访问，因此 Tine 拒绝使用它。
owner-protection-reason-lock = Tine 无法锁定此图谱的协调文件。
owner-protection-reason-handover = Tine 尚未能从另一个进程接管此图谱。
owner-protection-fix-permissions = 修复权限会将文件夹设为私有，不会替换任何锁文件。
owner-protection-fix-flatpak = 请运行以下命令，允许此 Flatpak 访问协调文件夹，然后重启 Tine 并选择“立即重试”。
owner-protection-fix-access = 请关闭其他 Tine 应用并停止所有 tine serve 进程。运行以下命令检查文件夹。请系统管理员以 0700 权限创建缺失的状态文件夹，并确保协调文件夹是属于您账户的真实目录，权限为 0700。不要删除或替换锁文件，然后选择“立即重试”。
owner-protection-fix-windows-access = 请关闭其他 Tine 应用并停止所有 tine serve 进程。运行以下命令选中协调文件夹。在“属性 → 安全 → 高级”中禁用继承，只允许您的账户、SYSTEM 和 Administrators 访问。文件夹不能是链接。保留现有锁文件，然后选择“立即重试”。
owner-protection-repair = 修复权限
owner-protection-retry = 立即重试
owner-protection-copy-command = 复制命令
owner-protection-copy-details = 复制诊断详情
owner-protection-details = 详情
owner-protection-status-failed = 无法检查保护状态。保留上次警告；请尝试“立即重试”。
owner-protection-retry-failed = 无法重试启用保护。请检查文件夹访问权限后重试。
owner-protection-copy-failed = 无法复制。请在详情中选中文字并手动复制。
root-recovery-recover = 恢复到此图谱
root-recovery-not-now = 暂不恢复
root-recovery-current-unsaved = 请先保存或解决此图谱中未保存的编辑。
root-recovery-failed = 无法恢复未保存的编辑：{ $error }
root-moved-notice = 图谱文件夹在打开期间被移动、替换或断开连接。未保存的编辑仍保留在此设备上。请关闭此窗口，然后从新位置重新打开图谱，或重新连接其所在的驱动器后再打开。
backup-old-location = 创建时图谱位于 { $path }。该图谱可能已移动或暂时不可用（例如，所在驱动器已断开连接）。
backup-delete-old = 删除此位置的备份
backup-delete-old-confirm = 删除图谱位于 { $path } 时创建的备份？该图谱可能已移动或暂时不可用（例如，所在驱动器已断开连接）。这些可能是断开连接的驱动器上某个图谱仅有的备份。这会从此设备移除这些本地快照。
backup-delete-old-failed = 无法删除此位置的备份：{ $error }
root-recovery-owner-held = 这些未保存的编辑仍由另一个 Tine 窗口、进程或正在完成保存的宿主持有。请关闭该图谱并等待保存完成，然后重试。
root-recovery-unavailable = 设备恢复区中已没有这些未保存的编辑。请重新打开图谱以再次检查。
root-recovery-lock-unavailable = Tine 无法为恢复操作独占此图谱。请关闭并重新打开图谱，然后重试。
root-recovery-moved-durability-unwitnessed = 未保存的编辑已移入当前图谱，但 Tine 无法确认这次移动已持久保存。请保持此窗口打开，并在关闭 Tine 前检查已恢复的编辑。
graph-open-other-app = 此图谱已在另一个 Tine 窗口中打开。
root-moved-draft-error = 图谱文件夹被移动、替换或断开连接，且无法写入恢复副本。请保持此窗口打开并复制未保存的文本，直到设备存储恢复正常。
