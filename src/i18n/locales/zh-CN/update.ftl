update-diagnostics = 诊断

update-download-manually = 手动下载

update-install-update = 安装更新

update-is-available-but-automatic-updates-are = { $product } { $version } 已发布，但此实验性 32 位 Windows 构建不支持自动更新。请手动下载 x86 软件包。

update-is-available-the-flatpak-cannot-update = { $product } { $version } 已发布。Flatpak 无法自行更新：请下载新的 .flatpak 包，再用 flatpak install --user 安装。

update-is-available-the-portable-build-cannot-update = { $product } { $version } 已发布。便携版不会自行更新：请下载新的便携版 ZIP，退出 Tine，再解压到同一文件夹。你的设置保存在其中的 tine-data 文件夹里。

update-couldn-t-restart-tine-quit-and-start = 无法重启 Tine（{ $error }）。请退出后重新启动。

update-downloading = 正在下载 { $product } { $version }…

update-the-update-was-downloaded-but-not-installed = 更新已下载但未安装，未保存的更改仍保留在窗口中。准备好后请再次选择“安装更新”。

update-is-available-you-re-on = { $product } { $version } 已发布，你当前使用 { $current }。

update-failure-notice = { update-failure-summary }诊断中会显示不含敏感信息的失败阶段；使用 --debug 启动 Tine 可查看已清理敏感信息的原因链。

update-failure-summary = { $phase ->
    [relaunch] 更新已安装，但 Tine 无法重新启动。
   *[other] 无法在{ update-failure-stage }阶段应用更新，正在打开发布页面。
    }

update-failure-stage = { $stage ->
    [manifest_fetch] 获取清单
    [manifest_parse] 解析清单
    [target_selection] 选择平台
    [download] 下载
    [signature_verification] 验证签名
    [install] 安装
   *[relaunch] 重新启动
    }
