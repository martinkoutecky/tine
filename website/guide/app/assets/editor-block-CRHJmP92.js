const e=`editor-block-click-to-zoom-shift-click-sidebar-ctrl-cmd-click-or-middle-click-new-tab-alt-click-other-pane-drag-to-move = 点击聚焦；Shift 点击 → 侧边栏；Ctrl/Cmd 点击或中键点击 → 新标签页；Alt 点击 → 另一窗格；拖动可移动

editor-block-open-block-references-shift-click-sidebar = 打开块引用（Shift 点击 → 侧边栏）

editor-block-couldn-t-load-templates = 无法加载模板：{ $detail }

editor-block-create = 创建“{ $name }”

editor-block-template = 模板：{ $name }

editor-block-create-tag = 创建 #{ $name }

editor-block-couldn-t-save-to-assets = 无法保存到 assets/：{ $detail }

editor-block-pasting-files = 正在粘贴文件…

editor-block-couldn-t-import-pasted-file = 无法导入粘贴的文件：{ $detail }

editor-block-couldn-t-save-pasted-file = 无法保存粘贴的文件：{ $detail }

editor-block-inserted-file = { $count ->
    [one] 已插入 { NUMBER($count, useGrouping: 0) } 个文件
   *[other] 已插入 { NUMBER($count, useGrouping: 0) } 个文件
    }

editor-block-skipped-item-folders-and-byte-only-files-over-64-mib-aren-t-pasted = { $count ->
    [one] 已跳过 { NUMBER($count, useGrouping: 0) } 项；文件夹和仅提供字节数据且超过 64 MiB 的文件无法粘贴
   *[other] 已跳过 { NUMBER($count, useGrouping: 0) } 项；文件夹和仅提供字节数据且超过 64 MiB 的文件无法粘贴
    }

editor-block-couldn-t-read-copied-files-use-upload-or-drag-and-drop-instead = 无法读取复制的文件，请改用上传或拖放

editor-block-couldn-t-capture-a-photo = 无法拍照（{ $detail }）

editor-block-couldn-t-import-the-photo = 无法导入照片（{ $detail }）

editor-block-couldn-t-save-the-recording = 无法保存录音（{ $detail }）

editor-block-couldn-t-import-the-recording = 无法导入录音（{ $detail }）

editor-block-couldn-t-start-recording = 无法开始录音（{ $detail }）

editor-block-recording-tap-the-mic-again-to-stop = 正在录音…再次点击麦克风即可停止

editor-block-recording-limit-reached-saving-the-captured-audio = 已达到录音上限，正在保存已录制的音频

editor-block-desktop-recording-save-error = 无法保存录音（{ $error }）

editor-block-another-voice-recording-is-already-active = 已有另一段录音正在进行

editor-block-recording-run-record-again-to-stop = 正在录音…再次运行 /record 即可停止

editor-block-couldn-t-access-the-microphone = 无法访问麦克风（{ $detail }）

editor-block-couldn-t-import-asset = 无法导入附件：{ $detail }

editor-block-couldn-t-open-draw-io = 无法打开 draw.io

editor-block-couldn-t-create-the-diagram = 无法创建图表：{ $detail }

editor-block-could-not-save-the-block-reference-resolve-the-page-save-and-try-again = 无法保存块引用。请解决页面保存问题后重试。

editor-block-could-not-save-the-block-reference = 无法保存块引用：{ $detail }

editor-block-plugin-result-was-not-inserted-because-the-block-changed-while-it-ran = 插件运行期间块发生了更改，因此未插入插件结果。

editor-block-plugin-slash-command-failed = 插件斜杠命令失败：{ $detail }

editor-block-outline-is-too-deep-to-insert = 大纲层级过深，无法插入

editor-block-couldn-t-read-the-clipboard = 无法读取剪贴板

editor-block-the-clipboard-doesn-t-hold-a-link-or-a-block-ref = 剪贴板中没有链接或块引用

editor-block-outline-is-too-deep-to-indent = 大纲层级过深，无法缩进

editor-block-pasted-text-is-too-large = 粘贴的文本过大

editor-block-pasting-image = 正在粘贴图片…

editor-block-pasted-outline-is-too-deep = 粘贴的大纲层级过深

editor-block-pasted-outline-was-refused = 粘贴的大纲被拒绝

editor-block-copy-result = 复制结果

editor-block-more-formatting = 更多格式
`;export{e as default};
