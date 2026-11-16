git-settings-label = 将更改提交到 git
git-settings-policy-off = 关闭
git-settings-policy-commit = 自动提交
git-settings-policy-sync-on-demand = 提交并按需同步
git-settings-policy-sync-automatically = 提交并自动同步
git-settings-interval = 最多每隔
git-settings-interval-aria = 两次自动提交之间的分钟数
git-settings-minutes = 分钟
git-settings-hint = 当图谱位于 git 仓库中时，Tine 会在一分钟没有编辑后提交图谱的更改（每个间隔最多一次），并在重命名、合并、删除或恢复备份的前后各提交一次。git 忽略的文件不会被提交。同步选项还会拉取并推送当前分支的上游：按需同步在你点击“立即同步”时进行，自动同步还会在每次提交后以及每个间隔进行一次。Tine 从不合并、变基或强制推送。
git-settings-not-available = 本设备暂不支持 git。
git-settings-not-a-repository = 此图谱不在 git 仓库中。你可以在这里创建仓库，保留图谱的历史记录。

git-clone-label = 从 git URL 克隆图谱
git-init-label = 将此图谱设为 git 仓库
git-init-remote = 空远程仓库的 HTTPS URL（可选）
git-init-pending = 图谱和首次提交已安全保存在本机。请完成设置，将首次提交推送到空远程仓库；如有需要，请先在下方登录。
git-init-finish = 完成仓库设置
git-setup-url = HTTPS 仓库 URL
git-setup-https = 请输入 HTTPS 仓库 URL，不要在其中包含用户名或令牌。
git-setup-off = 自动提交和同步仍为关闭状态。你可以在“设置 → 图谱 → Git”中开启。
git-setup-working = 正在处理…
git-clone-change-location = 选择其他位置
git-clone-open = 打开已克隆的图谱
git-clone-sign-in = 登录并重试克隆
git-clone-open-failed = 图谱已克隆到 { $path }。请重试打开，或稍后从图谱菜单打开。
git-init-failed = 无法完成创建图谱 git 仓库：{ $error }
git-settings-branch = 分支：{ $branch }
git-settings-unusable = Tine 无法在此提交：{ $message }
git-settings-last-commit = 上次提交 { $commit }，时间 { $time }。
git-settings-last-refused = 上次尝试失败：{ $message }
git-settings-repack-deferred = Git 清理正在等待；提交仍会继续。Tine 将自动重试。
git-settings-stale-lock = { $path } 已被占用超过 30 分钟，可能有 git 程序崩溃了。确认没有 git 命令在运行后请删除它。
git-settings-load-failed = 无法加载 git 设置：{ $error }
git-settings-save-failed = 无法更改 git 设置：{ $error }
git-settings-ignore-added =
    { $which ->
        [tine] Tine 已将 logseq/.tine-* 添加到 { $path }，以免将 Tine 在本设备上的自有文件（例如其回收站）提交到 git，这样同步同一仓库的多台设备不会在删除页面后停止同步。已在 git 中的副本会在一次单独的 Tine 提交中从 git 移除，但不会从磁盘删除。你可以删除这一行；Tine 不会在本设备上再次添加它，并且仍会将这些文件排除在其提交之外。
        [both] Tine 已将 .syncthing.* 和 logseq/.tine-* 添加到 { $path }，以免将 Syncthing 的临时文件和 Tine 在本设备上的自有文件（例如其回收站）提交到 git。已在 git 中的副本会在一次单独的 Tine 提交中从 git 移除，但不会从磁盘删除。你可以删除这些行；Tine 不会在本设备上再次添加它们。除非你自己的规则将其忽略，否则冲突副本仍会被提交。
       *[syncthing] Tine 已将 .syncthing.* 添加到 { $path }，以免将 Syncthing 的临时文件提交到 git。你可以删除这一行；Tine 不会在本设备上再次添加它。除非你自己的规则将其忽略，否则冲突副本仍会被提交。
    }
git-settings-ignore-failed =
    { $which ->
        [tine] Tine 无法确认是否已将 logseq/.tine-* 添加到图谱的 .gitignore。Tine 的提交仍会排除其自有文件。如果希望其他 git 工具也忽略它们，请检查该文件并手动添加这一行。
        [both] Tine 无法确认是否已将 .syncthing.* 和 logseq/.tine-* 添加到图谱的 .gitignore。Tine 的提交仍会排除临时文件和 Tine 的自有文件。如果希望其他 git 工具也忽略它们，请检查该文件并手动添加这些行。
       *[syncthing] Tine 无法确认是否已将 .syncthing.* 添加到图谱的 .gitignore。Tine 的提交仍会排除临时文件。如果希望其他 git 工具也忽略它们，请检查该文件并手动添加这一行。
    }
git-settings-remote = 远程仓库：{ $name }（{ $url }）
git-settings-sync-unusable = Tine 无法在此同步：{ $message }
git-settings-sync-now = 立即同步
git-settings-sync-now-failed = 无法开始同步：{ $error }
git-settings-signed-in = 已以 { $username } 身份登录 { $origin }。
git-settings-not-signed-in = 尚未登录 { $origin }。
git-settings-sign-in = 登录
git-settings-sign-out = 退出登录
git-settings-sign-in-hint = 请使用可读写该仓库的个人访问令牌：GitHub 在“Settings → Developer settings → Personal access tokens”，GitLab 在“Preferences → Access tokens”，Codeberg 在“Settings → Applications”。Tine 将其保存在本设备的私有文件中，绝不写入图谱。
git-settings-username = 用户名
git-settings-token = 令牌
git-settings-sign-in-save = 保存登录信息
git-settings-sign-in-cancel = 取消
git-settings-sign-in-failed = 无法登录：{ $error }
git-settings-sign-out-failed = 无法退出登录：{ $error }
git-sync-running = 正在同步…
git-sync-last-up-to-date = 上次同步于 { $time }：已是最新。
git-sync-last-pulled = 上次同步于 { $time }：已拉取新更改。
git-sync-last-pushed = 上次同步于 { $time }：已推送本设备的提交。
git-sync-last-failed = 上次同步于 { $time }，未完成。
git-sync-last-failed-since = 上次同步于 { $time }，未完成；上次完成的同步在 { $ok }。
git-sync-diverged = 本设备和远程仓库都有新的提交，因此 Tine 停止了同步，未做任何更改。请在 git 中合并：在终端运行下面的命令，解决冲突并提交，然后点击“立即同步”。（在 Tine 内合并将在以后提供。）
git-sync-auth-required = 远程仓库要求登录，或拒绝了 Tine 保存的令牌。请使用新令牌重新登录。
git-sync-network = Tine 无法连接远程仓库（{ $message }）。稍后会重试。
git-sync-held-unsaved = Tine 正在等待你在 { $path } 中的编辑保存后再更新它。下次同步时会重试。
git-sync-held-changed = 同步期间 { $path } 在 Tine 之外被修改，因此 Tine 没有更新任何内容。下次同步时 Tine 会提交该更改；由于远程仓库也有新提交，届时请在 git 中合并。
git-sync-held-changed-unnamed = 同步期间有文件在 Tine 之外被修改，因此 Tine 没有更新任何内容。下次同步时会重试。
git-sync-busy = 另一个 git 命令正在使用该仓库。Tine 会在下次同步时重试。
git-sync-incomplete = Tine 未能完成图谱的更新（{ $message }）。没有丢失任何内容：下次同步时会完成。
git-sync-pull-refused = Tine 无法更新 { $path }：{ $message }。未做任何更改。请在 git 中修复，然后点击“立即同步”。
git-sync-no-upstream = 该分支没有可同步的上游分支。请在 git 中设置，然后点击“立即同步”。
git-sync-refused = Tine 无法同步此仓库：{ $message }
git-sync-pull-stopped = Tine 在 { $path } 处停止更新图谱：{ $message }。部分文件已经更新。请在 git 中修复，然后点击“立即同步”。
git-sync-other = 同步已停止：{ $message }。请在 git 中修复，然后点击“立即同步”。
git-sync-other-path = 同步在 { $path } 处停止：{ $message }。部分文件可能已更新。请在 git 中修复，然后点击“立即同步”。
git-sync-fix-copy-command = 复制 git 命令
git-sync-fix-open-folder = 打开文件夹
git-sync-fix-sign-in = 重新登录
git-sync-copy-failed = 无法复制该命令。
git-sync-open-folder-failed = 无法打开文件夹：{ $error }
git-sync-notice-title = Git 同步需要处理
git-sync-notice-settings = Git 设置
git-sync-fix-line-endings = 为此仓库关闭行尾转换
git-sync-fix-line-endings-guide = 行尾：指南中的操作步骤
git-sync-line-endings-safe = Git 的自动行尾转换阻止了同步。所有已跟踪文件的字节都与索引中的内容一致，因此 Tine 可以在不更改任何图谱文件的情况下为此仓库关闭转换。
git-sync-line-endings-differ = 有 { $count } 个已跟踪文件的字节不同或无法读取，例如 { $path }。这可能是 Windows 行尾或未提交的编辑造成的。请先提交编辑，再按照指南步骤重新检出文件且不进行转换。
git-sync-line-endings-blocked = 关闭自动行尾转换后仍会阻止同步：{ $reason }。请按照指南步骤处理，然后重试。
git-sync-line-endings-failed = 无法关闭行尾转换：{ $error }
git-stale-lock-notice-title = Git 提交正在等待
git-sync-stale-lock = 锁文件 { $path } 自 { $time } 起没有变化，而 Tine 本身没有在使用 git。它很可能是 Tine 或其他 git 程序在提交过程中被中止时遗留的。在它被删除之前，Tine 无法提交。如果此刻没有 git 程序在这个仓库中工作，请删除它。
git-sync-fix-remove-stale-lock = 删除遗留的锁
git-sync-stale-lock-failed = 无法删除锁文件：{ $error }
