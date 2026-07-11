---
title: 设置参考
---

# 设置参考

当前 Koharu 的 Settings 页面主要包含以下 8 个区域：

- `Appearance`
- `Engines`
- `API Keys`
- `Typography`
- `AI`
- `Keybinds`
- `Runtime`
- `About`

本页基于当前应用实现说明这些设置项的实际行为。

## Appearance

`Appearance` 标签页当前包含：

- 主题：`Light`、`Dark`、`System`
- 从内置翻译资源中选择 UI 语言
- 用于渲染译文的 `Rendering Font`

主题、语言和渲染字体的变更都会在前端立即生效。

## Engines

`Engines` 标签页用于选择各个流水线阶段使用的后端：

- `Detector`
- `Bubble Detector`
- `Font Detector`
- `Segmenter`
- `OCR`
- `Translator`
- `Inpainter`
- `Renderer`

这些值会写入共享应用配置，并在修改时立即保存。

### 源文字策略

电商中文图片建议在“设置 → 引擎 → 源文字”选择“中文（推荐）”。该模式先用仅生成文本框的 Detector 建立内部候选，再由 PP-OCRv5 词框与 PaddleOCR-VL 原图裁剪结果共同确认中文目标：

- 纯英文单词或句子只参与内部候选判定，不形成可见文本框，也不进入完整 OCR、字体检测、分割、翻译、修补、智能排版或渲染阶段。
- 完整英文词与中文混排时，只公开和处理可安全分离的中文词框；英文区域始终从 Source 恢复。中文两侧的英文分别保存为互不跨越中文的保护区域。
- `S型曲线`、`A版` 这类单个拉丁字母与中文同排连续出现时作为一个中文目标处理；`S\n中文` 中独立成行的 `S` 保留不动。
- 多行中文被英文行隔开时，会生成多个独立的中文紧边界节点，后续 Font crop 和其他阶段不会包含中间英文。
- 可分框的 `AI` + `智能塑形` 只处理中文；如果 PP-OCRv5 只能返回无法分离的 `AI智能塑形` 单框，则整块安全跳过，不按字符比例猜测位置。
- 低置信度、字符对齐失败、坐标无效或无法安全分离时保留 Source，不擦除、不翻译、不渲染。
- 中文模式不支持同时执行检测和分割的复合 `comic-text-detector`；请改用 `pp-doclayout-v3`、`anime-text` 或 `comic-text-bubble-detector` 等仅生成文本框的 Detector。`all_text`（“全部文字”）兼容模式仍保留原有完整 CTD、节点级 OCR 和通用布局行为。

Han 脚本也用于部分日文汉字，因此仅凭字符脚本无法绝对区分纯汉字日文与中文；遇到此类素材应人工复核。

## API Keys

`API Keys` 标签页当前覆盖以下内置提供方：

- `OpenAI`
- `Gemini`
- `Claude`
- `DeepSeek`
- `DeepL`
- `Google Cloud Translation`
- `Caiyun`
- `OpenAI Compatible`

每个提供方都以折叠面板形式展示，并带有一个状态指示点：

- 绿色：已就绪（密钥已保存且发现成功）
- 琥珀色：缺少必需的配置项（API key，或 `OpenAI Compatible` 的 base URL）
- 红色：在已配置的端点上发现失败
- 灰色：尚未配置

当前行为：

- 提供方 API key 不会写入 `config.toml`
- 在关闭调试断言的 macOS 构建和 Windows 上，提供方 API key 存储在系统 keyring 中
- macOS 调试构建使用隔离的 `~/.koharu-dev/secrets/` 文件系统存储，并设置为仅所有者可访问
- 在 Linux 上，提供方 API key 存储在 `~/.koharu/secrets/` 的 Koharu 本地文件系统凭据存储中，并使用仅所有者可访问的文件权限
- 提供方的 `Base URL` 保存在共享应用配置中
- `OpenAI Compatible` 需要自定义 `Base URL`；模型列表通过对该 URL 调用 `GET /v1/models` 动态发现
- 机器翻译提供方（`DeepL`、`Google Cloud Translation`、`Caiyun`）只需要 API key；`Caiyun` 仅支持有限的目标语言
- 清除密钥会把它从凭据存储中删除

API 响应不会返回原始密钥，而是返回已遮罩的值。

Linux 和 macOS 调试构建的文件系统凭据存储依赖本地文件系统权限，而不是操作系统级加密。

## Typography

`Typography` 标签页用于为云端智能排版选择独立模型。它不保存第二份连接信息，而是复用 `API Keys > OpenAI Compatible` 中的 Base URL 和可选 API key；翻译模型仍由现有 LLM 选择与加载流程独立管理。

当前行为：

- 排版模型列表来自同一个 OpenAI-compatible `/v1/models` 动态目录
- 只有已配置 Base URL、模型目录就绪且所选模型仍存在时，才能启用自动排版
- 只切换排版模型或自动排版开关时会立即保存并更新当前配置，不会重新请求模型目录
- 修改共享 Base URL、保存或清除 API key 时会刷新模型目录
- 共享连接的修改会在下一次排版请求立即生效；已经加载的翻译 Provider 必须重新加载后才会使用新 URL 或 key
- 如果保存的排版模型已不在当前目录中，界面会标记该模型失效并阻止启用

Full Pipeline 和单个文本块的 `Generate` 只有在启用自动排版时才会自动加入排版阶段。画布工具栏的 `智能排版` 是独立手动入口；即使自动排版关闭，只要已配置共享 Base URL、有效排版模型和 Planner 引擎，它仍会对当前页运行排版并立即调用 Renderer。

排版失败不会阻断最终渲染。超时、连接/配额错误、缺少配置或无效模型输出会产生可见 warning；该页不会应用任何部分样式修改，Renderer 会继续使用当前译文和样式。

排版模型只能修改 Koharu 已支持的显式换行、字体、字号上限、颜色、粗斜体、描边和对齐。它不能修改字距、行高或文本框坐标，也不会自动下载字体。

## Keybinds

`Keybinds` 标签页可用于重新绑定工具切换、笔刷大小快捷键以及撤销/重做的按键。

当前行为：

- 选择 / 块 / 笔刷 / 橡皮 / 修复笔刷工具的默认按键分别为 `V`/`M`/`B`/`E`/`R`
- 笔刷大小步进的默认按键为 `[` 和 `]`
- 撤销与重做的默认按键为 `Ctrl + Z` 和 `Ctrl + Shift + Z`（macOS 上为 `Cmd + Z` 和 `Cmd + Shift + Z`）
- 画布缩放（`Ctrl` + 滚轮）、平移（`Ctrl` + 拖动）、全选（`Ctrl + A`）以及旧版 `Ctrl + Y` 重做备用方式不可重新绑定
- 编辑器中会高亮显示冲突；在同一界面也可以恢复默认值

快捷键偏好保存在前端 preferences 层中，而不是 `config.toml` 里。

完整的默认列表请参见 [键盘快捷键](keyboard-shortcuts.md)。

## Runtime

`Runtime` 标签页集中放置会影响共享本地运行时、且需要重启后生效的设置：

- `Data Path`
- `HTTP Connect Timeout`
- `HTTP Read Timeout`
- `HTTP Max Retries`

当前行为：

- `Data Path` 控制运行时包、下载模型、页面清单和图像 blob 的存储位置
- `HTTP Connect Timeout` 控制建立 HTTP 连接时的最长等待时间
- `HTTP Read Timeout` 控制读取 HTTP 响应时的最长等待时间
- `HTTP Max Retries` 控制遇到临时 HTTP 故障时的自动重试次数
- 这些 HTTP 值会应用到下载和提供方请求共用的运行时 HTTP 客户端
- 由于这些值在启动时加载，应用变更时会先保存配置，再重启桌面应用

## About

`About` 标签页当前显示：

- 当前应用版本
- 是否存在更新的 GitHub release
- 作者链接
- 仓库链接

在打包应用模式下，版本检查会把本地版本与 `mayocream/koharu` 的最新 GitHub release 进行比较。

## 持久化模型

当前设置数据分布在多个存储层中：

- `config.toml` 保存 `data`、`http`、`pipeline` 以及提供方 `baseUrl` 等共享配置
- 提供方 API key 通过上文所述的平台凭据存储与 `config.toml` 分开保存
- 主题、语言和渲染字体存储在前端 preferences 层中

因此，清除前端 preferences 并不等于清除已保存的提供方 API key 或共享运行时配置。

## 相关页面

- [使用 OpenAI 兼容 API](../how-to/use-openai-compatible-api.md)
- [模型与提供方](../explanation/models-and-providers.md)
- [HTTP API 参考](http-api.md)
