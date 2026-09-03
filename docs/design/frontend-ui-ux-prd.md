# Symbio 前端 App 整体 UI/UX 系统化优化 PRD

| 项目 | 内容 |
| --- | --- |
| 文档语言 | 简体中文 |
| 项目名 | `symbio_frontend_ui_ux_systematization` |
| 技术栈 | Vue 3 (SFC) + Pinia + vue-router + Vite 8 + Tauri 2 |
| 前端根目录 | `D:\Bing\symbio\tauri`（下文所有 `文件:行` 均相对 `src/`） |
| 版本 | v1.0（2026-09-02） |
| 作者 | 许清楚（产品经理） |
| 状态 | 待用户拍板「待确认问题」后进入架构设计 |

## 0. 原始需求复述

> 「用类似思路，系统化的规划也优化一下整个前端 App 的 UI/UX，确保整体简洁、大方、美观。」

「类似思路」= 已完成**会话区改造**所确立的范式，本项目需将其**推广至全 App**，且**不得回退**：

| # | 范式 | 会话区现状 | 推广目标 |
| --- | --- | --- | --- |
| ① | 语义令牌驱动、浅色/深色自动适配 | ✅ 已建立 `--color-*` 会话区令牌 | 全 App 100% 令牌化 |
| ② | 去掉过度装饰（无框扁平；内容靠节点头/标题分组而非靠框） | ⚠️ 部分（残留渐变/光晕） | 全量清理装饰性渐变/多重描边 |
| ③ | 间距与层次清晰、紧凑不稀疏 | ⚠️ 会话区已有 `--msg-gap` 等，但定义在组件局部 | 全局 `--space-*` 阶梯 |
| ④ | 对齐 Claude / Codex 类现代 agent 应用的极简观感 | ⚠️ 部分 | 全 App 统一 |

---

## 1. 产品目标与非目标

### 1.1 产品目标（5 条，可衡量）

| ID | 目标 | 衡量方式 |
| --- | --- | --- |
| G1 | **深浅色零硬编码**：任意页面在浅色/深色下均正确渲染，无刺眼白块 / 不可见 hover / 低对比文本 | 硬编码色值残留 = 0（白名单见 §7.2）；14 张关键页面截图人工核对通过 |
| G2 | **视觉语言统一**：同类元素（卡片、列表项、图标按钮、状态点、标签、对话框、Toast）在各页面表现一致 | 8 类组件的重复定义收敛为 1 份；`gap`/`border-radius` 取值落入规定阶梯 |
| G3 | **去装饰、扁平极简**：删除全部装饰性渐变、光晕、多重描边、冗余阴影 | 装饰性渐变 6 处 → 0；「阴影+1px 描边」二重奏 4 处 → 0 |
| G4 | **交互态与可达性齐全**：所有可交互元素具备 hover / active / focus-visible / disabled 四态；键盘可完成全部主要任务 | `:focus-visible` 覆盖 100%；卡片可 Tab + Enter/Space 选中；纯图标按钮 100% 有 `aria-label` |
| G5 | **可读性达标**：正文对比度 ≥ 4.5:1，关键语义色不承载装饰 | §4.6 列出的全部前景/背景组合通过对比度校验（当前有 4 组已知不达标） |

### 1.2 非目标（明确不做）

| 不做 | 说明 |
| --- | --- |
| ❌ 不改后端 | 不触碰 Rust / Tauri 插件 / 任何 IPC 契约 |
| ❌ 不改业务功能逻辑 | 不改数据流、不改 Pinia store 的语义、不改服务层调用；仅调整展示与样式 |
| ❌ 不重构数据流 | 不迁移状态管理方案，不重写 composable 的对外契约 |
| ❌ 不引入新 UI 框架 / 组件库 | 不使用 Element Plus / Naive UI / Tailwind 等；**仅用现有 Vue 3 + CSS 自定义属性令牌实现** |
| ❌ 不改信息架构主干 | 6 个页面的路由与职责不变（侧边栏是否加文字标签属于「待确认」，确认后才动） |
| ❌ 不做功能新增 | 本次是视觉/体验改造，不新增业务能力 |

---

## 2. 用户故事

| ID | 用户故事 | 关联目标 |
| --- | --- | --- |
| US1 | 作为**深色主题用户**，我希望打开任意页面（会话 / 设置 / Agent / Skill / MCP / Model Provider / 文件查看器）都清晰一致，这样我不需要为了让界面可用而切回浅色 | G1, G5 |
| US2 | 作为**配置管理员**，我希望在 Model Provider 页配置 API Key / 协议 / 超时时不觉得杂乱，字段分组与层级一眼可辨，这样我不会填错或漏填 | G2, G3 |
| US3 | 作为**键盘重度用户**，我希望只用 Tab / Enter / Esc 就能在会话列表、资源列表、设置项之间切换与确认，这样我不必依赖鼠标 | G4 |
| US4 | 作为**窄窗口用户**（窗口宽度 < 900px），我希望三栏会话布局仍然可用（侧栏可收起或自动降级），这样内容区不会被挤压到不可读 | G2 |
| US5 | 作为**首次使用者**，我看到 48px 纯图标侧边栏时能立刻理解每个图标的含义，这样我不需要逐个 hover 试探 | G4 |
| US6 | 作为**日常使用者**，我希望「选中 / 运行中 / 出错 / 已停用」这些状态在所有列表里用同一种视觉表达，这样我不需要在每个页面重新学习 | G2 |
| US7 | 作为**长时间阅读 AI 输出的用户**，我希望正文、代码、引用、工具调用之间有稳定的间距与层级，这样长对话不会视觉疲劳 | G3, G5 |
| US8 | 作为**无障碍用户**（低视力 / 使用读屏软件），我希望纯图标按钮有可读名称、焦点位置可见、正文对比度达标，这样我能独立使用本应用 | G4, G5 |

---

## 3. 现状审计与问题清单

### 3.0 审计方法与样本

- **样本**：`src/` 下 **38 个 `.vue` 文件**，合计 **14,472 行**；另有 `stores/appearance.ts`、`router/index.ts` 作为上下文。
- **方法**：逐文件通读 + 全量 grep 取证（`#[0-9a-fA-F]{3,6}`、`rgba(0, *0, *0`、`rgba(255, *255, *255`、`linear-gradient|radial-gradient`、`border-radius`、`font-size`、`focus-visible|aria-label|role=|tabindex`、`var\(--`。
- **关键结构性事实**：
  - `find src -name "*.css"` → **0 个结果**。**项目没有任何全局样式表**，`App.vue` 的 `<style>`（非 scoped）是事实上的全局样式入口，且仅含 reset + 令牌定义，**无组件级基础样式**。
  - 除 `App.vue` 外，硬编码十六进制色值 **299 处**，分布在 33 个文件。
  - `rgba(0, 0, 0, …)` 叠加 **81 处**，分布在 28 个文件。

### 3.1 令牌底座问题（P0，影响面最大）

| # | 问题 | 证据 | 影响 |
| --- | --- | --- | --- |
| T1 | **4 个令牌只在深色块定义，浅色完全缺失** | `--color-input-bg` / `--color-surface-strong` / `--color-hover-bg` / `--color-active-bg` 仅见于 `App.vue:167-170`（dark 块） | 浅色下 4 个组件全部走 `var()` fallback：`ChatSettings.vue:550,551,601,612`、`ChatInputArea.vue:217,226`。**浅色主题实际上没有令牌驱动的 hover/input 表面**，全靠硬编码 fallback 兜底 |
| T2 | **5 个令牌被引用但从未定义** | `--color-border-subtle`（`McpView.vue:407,448`、`ModelProvidersSettings.vue:509`、`ModelProviderCard.vue:104`、`McpServerCard.vue:113`、`McpServerSettings.vue:709`）<br>`--font-mono`（`McpView.vue:425`、`ModelProvidersSettings.vue:452`、`ModelProviderCard.vue:211,234`、`McpServerCard.vue:216`、`McpServerSettings.vue:572,751,771`）<br>`--color-primary-hover`（`ModelProvidersSettings.vue:706`）<br>`--color-text-primary`（`HomedirSwitcher.vue:239,275,306`，**且无 fallback**）<br>`--border-color`（`ExplorerPage.vue:655,744`） | 全部退化为 fallback；`--color-text-primary` 无 fallback → `color` 计算为 unset（继承），行为不可预测 |
| T3 | **3 个令牌定义后从未使用（死令牌）** | `--header-height`（`App.vue:30`）、`--color-msg-card-border`（`App.vue:37,129`）、`--card-pad-y`（`ModelChatPanel.vue:558`） | 误导后续维护者 |
| T4 | **同一令牌两处冲突定义** | `--msg-gap`：`App.vue:38,130` = `0.45rem`；`ModelChatPanel.vue:556` = `0.4rem` | 组件内覆盖全局，节奏不唯一 |
| T5 | **`--color-code-bg/fg` 令牌存在但只有 1 个组件用** | 仅 `MessageNode.vue:860-861, 1118-1119, 1282-1283` 使用 | `CodeEditor.vue:174,194,212`（`#2d2d2d/#1e1e1e/#1a1a1a`）、`CodeBlockExecutor.vue:166,199`（`#1e1e1e`）、`ExplorerPage.vue:751-752`（`#1e1e1e/#d4d4d4`）各写一套硬编码编辑器配色 |
| T6 | **滚动条硬编码** | `App.vue:108`（`#ccc`）、`App.vue:113`（`#bbb`） | 深色下滚动条为浅灰，与深色背景割裂 |

### 3.2 硬编码浅色 / 深色未适配（按模块）

> **Top 危险区**：深色主题下会直接出现「刺眼白块」或「文字不可见」。

#### 3.2.1 会话区（改造未完成的残留）

| 文件:行 | 硬编码 | 深色下表现 |
| --- | --- | --- |
| `chat/ChatContextBar.vue:240` | `background: rgba(255, 255, 255, 0.85)` | 白色代码预览块浮在深色卡片上 |
| `chat/ChatContextBar.vue:263` | `color: #1f2937` | 深色文本 → 近黑不可读 |
| `chat/ChatContextBar.vue:277` | `linear-gradient(to bottom, transparent, rgba(255,255,255,0.95))` | 白色渐隐遮罩 → 与深色背景冲突 |
| `chat/ChatContextBar.vue:175` | `color: #7c3aed` | 紫字在深底对比不足 |
| `chat/ChatSettings.vue:531` | `.menu { background: white; }` | **整个下拉菜单为纯白块** |
| `chat/ChatInputArea.vue:261` | `.send-btn.stop-btn { background: #dc3545; }` | Bootstrap 红，与 `--danger` 体系无关 |
| `session/SessionCard.vue:220,226,231,235` | `#22c55e / #f59e0b / #ef4444 / #cbd5e1` | 状态点未令牌化 |
| `session/ChatMainPanel.vue:200` | `color: #22c55e` | 「AI 处理中」未令牌化 |
| `MessageNode.vue:1011,1035,1241,1306` | `#dc2626 / #ef4444 / #94a3b8 / #16a34a` | 残余硬编码 |

#### 3.2.2 设置页（深色最严重）

| 文件:行 | 硬编码 | 说明 |
| --- | --- | --- |
| `SettingsPage.vue:376` | `.nav-item:hover { background: #f0f0f0; }` | 深色下 hover 出现亮灰块 |
| `SettingsPage.vue:377` | `.nav-item.active { background: #e8e8f0; }` | 同上 |
| `SettingsPage.vue:387` | `.setting-item:hover { background: #fafafa; }` | 同上 |
| `SettingsPage.vue:384-385` | `.message.success { background:#d4edda; color:#155724 }` / `.error { #f8d7da / #721c24 }` | Bootstrap 3 时代配色，深色完全不可用 |
| `SettingsPage.vue:458` | `.dialog { background: white; }` | **纯白对话框** |
| `SettingsPage.vue:398-399` | `.provider-pill { #eef2ff / #4338ca }` | 硬编码 |
| `SettingsPage.vue:411-412` | `.toggle-slider { background:#ccc }` / `::before { background: white }` | 开关在深色下为亮灰轨道 + 白滑块 |
| `SettingsPage.vue:439` | `.recent-item { background: #f5f5f5 }` | 硬编码 |
| `SettingsPage.vue:447` | `.server-info code { background: #f0f0f0 }` | 硬编码 |
| `SettingsPage.vue:450-451` | `.icon-btn:hover { #f0f0f0 }` / `.danger:hover { #fee }` | 硬编码 |
| `SettingsPage.vue:467` | `.action-btn.secondary { background: #f0f0f0 }` | 硬编码 |

#### 3.2.3 四个资源页 / 公共骨架

| 文件:行 | 硬编码 | 说明 |
| --- | --- | --- |
| `common/ResourceShell.vue:194` | `background: rgba(102, 126, 234, 0.04)` | 主色硬编码 rgba；深色主色是 `#818cf8`，不匹配 |
| `common/ResourceShell.vue:205` | `.running-pulse { background: #22c55e }` | 未令牌化 |
| `common/ResourceShell.vue:177` | `.icon-btn:hover { rgba(0,0,0,0.06) }` | 深色不可见 |
| `common/ResourceCard.vue:93,99,157,189-193` | `rgba(102,126,234,.05)` / `rgba(102,126,234,.1)` / `#22c55e / #f59e0b / #3b82f6 / #6b7280` | 主色与语义色硬编码 |
| `common/ResourceCard.vue:128-132` | `#9ca3af / #22c55e / #f59e0b / #ef4444 / #d1d5db` | 状态点色板 |
| `views/AgentView.vue:221,246,287-289` | `rgba(0,0,0,0.04)` / `#fff` / `#333` | 代码块 + Toast |
| `views/SkillView.vue:313-317,350,394-395,410,415,445-454` | 主色/语义色 rgba 系列 + `#b45309 / #b91c1c` | 徽章、代码、标签 |
| `views/McpView.vue:309,311,338-339,349-353,432,444,462-470` | `#fff` / `#22c55e / #ef4444` | Toast、测试结果卡 |
| `views/ModelProvidersView.vue:223-228` | `#fff` / `#22c55e`（**默认底色**）/ `#ef4444` / `#4f46e5` | Toast 与其他页不一致 |
| `settings/ModelProviderCard.vue:114` | `.active { border-left-color: #22c55e }` | **选中态用绿色** |
| `settings/McpServerCard.vue:122-123,150,155,160,258,264,269` | 主色 rgba / `#94a3b8 / #22c55e / #16a34a / #d97706` | 见 §3.6 |
| `settings/ModelProvidersSettings.vue:465,541,565` | `#15803d / #dc2626 / #4f46e5` | 徽章、必填标记 |

#### 3.2.4 弹层 / 编辑器 / 文件查看器

| 文件:行 | 硬编码 | 说明 |
| --- | --- | --- |
| `ModelSelectionDialog.vue:307` | `background: rgba(255,255,255,0.98)` | **悬浮 AI 对话框近乎纯白**，深色下严重刺眼 |
| `ModelSelectionDialog.vue:337,349,359,376,382,438,444,447` | `#888 / #1a1a1a / #999 / #888 / #444 / #999 / #999` | 整枚组件无令牌 |
| `common/ConfirmDialog.vue:143,205` | `background: var(--color-bg)` | 浅色下 `--color-bg = #f5f5f5`（页面灰）→ 对话框与页面同色，无浮起感 |
| `common/ConfirmDialog.vue:198,170,173-175` | `rgba(0,0,0,0.02)` / `rgba(0,0,0,0.04)` / 语义 rgba | 深色不可见 |
| `common/HomedirSwitcher.vue:286,289` | `color: #d33` / `rgba(221,51,51,0.08)` | 第 7 种红色 |
| `session/SessionSettingsDialog.vue:316,320,341,353,410-413` | `#cbd5e1 / #ef4444 / #d1d5db / #fff / #16a34a / #d97706 / #dc2626 / #4f46e5` | 心跳图标、开关、状态提示条 |
| `CodeEditor.vue:174,175,183,194,195,208,212` | `#2d2d2d / #858585 / #3c3c3c / #1e1e1e / #d4d4d4 / #666 / #1a1a1a` | 编辑器在浅色主题下**仍然是深色**；行号 `#666` on `#2d2d2d` 对比不足 |
| `ExplorerPage.vue:540,559,595,618,631,704,751-752,844` | `#f0f0f0 ×4 / #dc2626 / #f5f5f5 / #f59e0b / #1e1e1e / #d4d4d4` | 见 §3.10（该文件为死代码，但 `FileTreeNode` 被复用） |
| `FileTreeNode.vue:166,170` | `#f0f0f0` / `#e8e8f0` | **树节点 hover/选中硬编码浅色**，深色不可用 |
| `CodeBlockExecutor.vue:123,166,177,181,199,209` | `#6c757d / #1e1e1e / #f8f9fa / #fff5f5 / #f87171` | 死代码 |
| `Diagnostic.vue:78,89,101` | `#f5f5f5 / white / #fff` | 死代码 |
| `MarkdownEditor.vue:286,300,307` | `#fff / #e5e5e5 / #1f1f1f / #fff` | 编辑器与浮动提示硬编码 |

### 3.3 装饰过重（需依据「去装饰」原则清理）

| 类型 | 位置 | 判定 |
| --- | --- | --- |
| 装饰性渐变 | `MainLayout.vue:210`（logo `linear-gradient(135deg, primary, primary-dark)`） | ✗ 删除 → 纯色品牌块 |
| 装饰性渐变 | `chat/ChatContextBar.vue:127`（卡片底 `135deg` 紫→蓝 rgba） | ✗ 删除 → 扁平卡片 |
| 装饰性渐变 | `chat/ChatContextBar.vue:142`（展开态再次渐变） | ✗ 删除 |
| 装饰性渐变 | `ModelSelectionDialog.vue:364`（`.selected-context` 紫→蓝渐变） | ✗ 删除 |
| 装饰性渐变 | `session/SessionCard.vue:187,191,195`（working/waiting/failed 三态横向渐变） | ✗ 改为扁平 `background` + 左侧 2px 语义色条 |
| 功能性渐变 | `SkillView.vue:430-431`（`mask-image` 折叠渐隐）、`ChatContextBar.vue:277`（渐隐遮罩） | ✅ **保留**（承载信息），仅把白色改为令牌 |
| 阴影 + 1px 描边二重奏 | `chat/ChatSettings.vue:533`、`ModelSelectionDialog.vue:309`、`McpView.vue:339`（双阴影） | ✗ 统一为 `box-shadow` 单层级 + `border` |
| 状态点光晕 | `session/SessionCard.vue:222`（`box-shadow: 0 0 4px rgba(34,197,94,0.5)`） | ✗ 删除 |
| 状态点脉冲环 | `settings/ModelProviderCard.vue:147,160-161`、`settings/McpServerCard.vue:156,165-166`（`box-shadow: 0 0 0 4px` 扩散环） | ✗ 统一为 `opacity` 呼吸（与 `ResourceShell.vue:207` / `SessionListPanel.vue:164` 一致） |
| 强调色竖条 | `McpView.vue:349,353`（`border-left: 4px solid`） | ✗ 降为 2px 或改用语义色徽章 |
| 失效的毛玻璃 | `ModelSelectionDialog.vue:314`（`backdrop-filter: blur(12px)` + `0.98` 不透明底） | ✗ 删除（背景不透明，模糊无效果） |
| 装饰性缩放 | `chat/ChatInputArea.vue:258-259`（`transform: scale(1.05) / scale(0.95)`） | ✗ 删除，改用背景/阴影变化 |

### 3.4 间距 / 圆角 / 字号 / 阴影 / 动效 不统一

| 维度 | 实测分布 | 结论 |
| --- | --- | --- |
| **圆角** | `6px×52`、`4px×35`、`8px×31`、`50%×20`、`3px×11`、`12px×7`、`999px×6`、`10px×5`、`5px×4`、`16px×2`、另有 `24px / 22px / 1px / 18px / 14px` 各 1，以及 `MessageNode.vue:1063` 的 `14px 14px 4px 14px` 非对称值 | **16 种取值，无 `--radius-*` 令牌** |
| **`gap`** | `0.5rem×41`、`0.4rem×24`、`0.3rem×14`、`0.25rem×12`、`0.75rem×10`、`1rem×6`、`0.35rem×6`、`8px×4`、`4px×4`、`0.6rem×4`、`2px×3`、`0.2rem×3`、`0.85rem×2`、`0.65rem×2` | **14 种取值，含 rem/px 混用** |
| **`padding`** | 至少 **30+ 种**不同组合（`1rem×20`、`0.75rem 1rem×11`、`0.5rem 1rem×10`…`0.05rem 0.4rem×3`、`6px 10px×2`、`10px 14px×2`） | 完全无阶梯 |
| **字号单位** | `rem` 约 250 处 vs **`px` 16 处**：`ChatContextBar.vue:161,171,174,204,261,285`、`ModelSelectionDialog.vue:342,347,358,375,381,415`、`ChatInputArea.vue:291`、`MarkdownEditor.vue:289,291,307` | **px 字号不受「小/中/大」字号档位影响**（`stores/appearance.ts:27-31,61` 通过改 `<html> font-size` 缩放 rem）。且 `ChatContextBar` 出现 **9px / 10px** 字号，低于可读下限 |
| **图标按钮尺寸** | `22px`（`SessionCard.vue:337-338`、`ChatContextBar.vue:221-222`、`ModelSelectionDialog.vue:353-354`）、`24px`（`SessionExplorerPanel.vue:192-193`、`ExplorerPage.vue:527-528`）、`26px`（`ResourceShell.vue:166-167`、`SessionListPanel.vue:128-129`）、`28px`（`ChatMainPanel.vue:212-213`、`SessionSettingsDialog.vue:282-283`）、`32px`（`ExplorerPage.vue:606-607`） | **5 种尺寸（22/24/26/28/32）**；22px 低于 WCAG 2.2 AA 目标尺寸 24px 下限 |
| **过渡时长** | `0.12s×14`、`0.15s×30`、`0.18s×3`、`0.2s×32`、`0.25s×1`、`0.3s×3`、`0.4s×1` | 7 种，无 `--motion-*` |
| **阴影** | 至少 8 种不同写法，强度从 `0 1px 2px rgba(0,0,0,0.08)` 到 `0 20px 50px rgba(0,0,0,0.25)` | 无层级定义 |

### 3.5 对比度与可读性（实测计算，sRGB 相对亮度 + WCAG 2.1 公式）

| # | 前景 / 背景 | 实测对比度 | 达标？ | 使用位置 |
| --- | --- | --- | --- | --- |
| C1 | `#999999`（`--color-text-muted` 浅色，`App.vue:26`）on `#ffffff` | **2.85 : 1** | ❌ 需 ≥4.5 | 全局次要文本：`.setting-desc`、`.card-meta`、`.preview-text`、`.activity-text`、`.no-selection`、`.empty-state` 等 |
| C2 | `#64748b`（`--color-text-muted` 深色，`App.vue:124`）on `#1e1e2e`（`--color-bg` 深色） | **3.45 : 1** | ❌ 需 ≥4.5 | 同上（深色） |
| C3 | `#ffffff` on `#667eea`（`--color-primary` 浅色，`App.vue:19`） | **3.66 : 1** | ❌ 需 ≥4.5 | 全部主按钮白字：`SettingsPage.vue:415`、`.confirm-btn.primary`（`ConfirmDialog.vue:224`）、`.hb-btn.primary`（`SessionSettingsDialog.vue:439`）、`.send-btn`（`ChatInputArea.vue:257`）、`.btn-primary`（`HomedirSwitcher.vue:319`）、`.action-btn`（`ChatSettings.vue:610`）、`.load-error-btn.primary`（`ChatMainPanel.vue:333`） |
| C4 | `#667eea` 作为**前景文字** on `#ffffff` | **3.66 : 1** | ❌ 需 ≥4.5 | `.seg-btn.active`（`SettingsPage.vue:420`）、`.check`（`ChatSettings.vue:558`）、`.activity-text.kind-primary`（`ResourceCard.vue:157`）、`.advanced-toggle:hover`（`ModelProvidersSettings.vue:565`）、`.preview-chip`（`SettingsPage.vue:427`） |
| C5 | `rgba(0,0,0,0.04~0.06)` 作为 hover 叠加 on 深色表面 | **≈1.0 : 1（无变化）** | ❌ 功能性失效 | 81 处中的深色路径，如 `ResourceShell.vue:177`、`SessionListPanel.vue:139`、`MainLayout.vue:249` |
| C6 | `#666`（`--color-text-secondary` 浅色）on `#f5f5f5` | 5.26 : 1 | ✅ | — |
| C7 | `#333`（`--color-text` 浅色）on `#f5f5f5` | 11.59 : 1 | ✅ | — |
| C8 | `#b91c1c` on `#fef2f2`（错误卡） | 5.91 : 1 | ✅ | `App.vue:58,56` |

> **结论**：令牌层本身有 **2 组色值不达标（C1/C2）**，`--color-primary` 作为填充底与作为前景字**双双不达标（C3/C4）**。这是本 PRD 建议把 `--accent` 从 `#667eea` 调整为 `#4f46e5`（白字 6.29:1、作前景字 6.29:1）的直接依据。

### 3.6 状态与反馈：空 / 加载 / 错误 / Toast

| 问题 | 证据 |
| --- | --- |
| **Toast 4 份实现且不一致** | `.toast` 定义于 `AgentView.vue:279`、`McpView.vue:301`、`ModelProvidersView.vue:213`、`SkillView.vue:468`。其中 **`ModelProvidersView.vue:225` 默认底色为 `#22c55e`（绿）**，其余三处为 `#333`（深灰）；`info` 色两派：`#3b82f6` vs `#4f46e5`；`ModelProvidersView` 无 `.toast.success` 规则 |
| **空状态 2 份实现** | `.empty-state` 定义于 `ResourceShell.vue:216`、`SessionListPanel.vue:175`；`ChatMainPanel.vue:239`、`.empty-chat`（`ModelChatPanel.vue:563`）、`.empty-explorer`（`SessionExplorerPanel.vue:227`）、`.empty-file/.empty-content`（`ExplorerPage.vue:767`）各写一份 → **6 种空状态** |
| **加载态** | 均为纯文案「加载中…」（`ResourceShell.vue:74`、`ChatMainPanel.vue:53`、`FileViewerWindow.vue:11`、`SessionExplorerPanel` 无独立加载态）、或 spinner（`FileViewerWindow.vue:215`、`ExplorerPage.vue:789`）。**无骨架屏** |
| **错误态** | 4 种表达：`ChatMainPanel.vue:272`（带图标+重试的完整错误页）、`FileViewerWindow.vue:14`（纯文案）、`SessionExplorerPanel.vue:268`（`#ef4444` 纯文案 + 重试按钮）、`ExplorerPage.vue:558`（`#dc2626`）。`CodeBlockExecutor.vue:181`（`#fff5f5`）为第 5 种 |
| **原生浏览器弹窗 8 处** | `SessionListPanel.vue:84,87,92`（alert/confirm/alert）、`ChatMainPanel.vue:143`（`prompt` 重命名）、`ChatMainPanel.vue:151`（`window.confirm` 清空历史）、`AgentView.vue:146`（confirm 删除）、`ModelProvidersView.vue:176`（confirm 删除）、`FileViewerOverlay.vue:352`（alert）、`ChatSettings.vue:440`（alert）。**项目已有 `ConfirmDialog.vue` 却未统一使用**（仅 `McpView.vue:100` 用了） |

### 3.7 交互态缺失（hover / active / focus / disabled）

| 状态 | 覆盖率 | 证据 |
| --- | --- | --- |
| `hover` | 较好，但 **81 处用 `rgba(0,0,0,…)`** 硬编码，深色下失效 | 见 §3.5 C5 |
| `active` / `pressed` | **差**。仅 `ChatSettings.vue:551`、`.send-btn:active`（`ChatInputArea.vue:259`）、`.dir-item`（无）等零星几处 | 多数按钮点击无即时反馈 |
| `focus-visible` | **0 处** | `grep -rn "focus-visible" src` → **0 结果** |
| `focus`（非 visible） | 11 处，全部在表单输入：`CodeEditor.vue:211`、`HomedirSwitcher.vue:279`、`MessageNode.vue:1388`、`SessionSettingsDialog.vue:387-388`、`McpServerSettings.vue:785-786`、`ModelProvidersSettings.vue:604-607` | **按钮 / 卡片 / 图标按钮 / 导航项全部无焦点指示** |
| `disabled` | 部分。`.icon-btn:disabled`（`ResourceShell.vue:181`）、`.btn-danger:disabled`（`AgentView.vue:253`）、`.confirm-btn:disabled`（`ConfirmDialog.vue:216`）等有；但多数按钮无 disabled 视觉 |

> **这是本次审计中最严重的单项缺陷**：一个 38 组件的应用，`focus-visible` 覆盖率为 **0**，键盘用户无法判断焦点位置。

### 3.8 键盘可达性与 ARIA

| 指标 | 实测 |
| --- | --- |
| `aria-label` | **4 处**（`ConfirmDialog.vue:22`、`HomedirSwitcher.vue:16,19`、`MainLayout.vue:89`） |
| `role="…"` | **4 处**（`ConfirmDialog.vue:21` alertdialog、`HomedirSwitcher.vue:16` dialog、`FileViewerOverlay.vue:6` dialog、`ModelChatPanel.vue:5` alert） |
| `tabindex` | **1 处**（`ConfirmDialog.vue:24`） |
| `:focus-visible` | **0 处** |

具体问题：

1. **列表卡片不可键盘操作**：`ResourceCard.vue:12-16`、`SessionCard.vue`（`onClick`）、`ModelProviderCard.vue:92-94`、`McpServerCard.vue:101-103` 均为 `<div @click>`，无 `role="button"` / `tabindex="0"` / `@keydown.enter|space` / `aria-selected`。→ **键盘用户完全无法切换会话 / 选择 Provider / 选择 MCP Server**。
2. **侧边栏图标按钮缺 `aria-label`**：`MainLayout.vue:8-81` 的 6 个导航按钮只有 `title`，唯独底部「系统目录」按钮（`MainLayout.vue:89`）有 `aria-label` → **不一致**。
3. **纯图标按钮普遍缺可读名称**：`ResourceShell.vue:37-43`（新建）、`AgentView.vue:17-22` / `SkillView.vue:22-27`（刷新）、`ChatMainPanel.vue:9,18`（清空/重命名）、`SessionSettingsDialog.vue:21`（`×`）、`HomedirSwitcher.vue:19`（`×`，唯一有）。
4. **开关无焦点指示**：`SettingsPage.vue:410`（`opacity:0; width:0; height:0`）、`SessionSettingsDialog.vue:337` 同款 → 仍可聚焦但焦点不可见。
5. **`ConfirmDialog` 文档与实现不符**：头部注释（`ConfirmDialog.vue:8`）声明「focus trap、ESC 关闭、aria 属性」，实际仅 `dialogRef.focus()`（`:119-127`）与 `@keydown.esc`（`:16`，挂在不可聚焦的 overlay 上）；**无焦点陷阱**，Tab 可移出对话框。
6. **目标尺寸不足 24px**：`SessionCard.vue:337-338`（22px 删除按钮）、`ChatContextBar.vue:221-222`（22px）、`ModelSelectionDialog.vue:353-354`（22px）。

### 3.9 图标风格不统一

| 问题 | 证据 |
| --- | --- |
| **emoji 与 inline SVG 混用** | emoji 共 **140 处 / 19 个文件**：`ExplorerPage.vue:42`、`FileTreeNode.vue:40`、`MessageNode.vue:13`、`ChatSettings.vue:12`、`SettingsPage.vue:5`（导航项 🎨💬🔧🌐ℹ️）、`McpView.vue:4`（🗑 ✓ ✗）、`ChatContextBar.vue:4`（✨📄📍）、`WorkdirPicker.vue:3`、`ModelSelectionDialog.vue:3`、`CodeBlockExecutor.vue:3`、`FileViewerOverlay.vue:3`、`SkillView.vue:2`（⚠）等。<br>而 `MainLayout.vue`、`ResourceShell.vue`、`SessionListPanel.vue`、`ChatMainPanel.vue` 使用 **Feather/Lucide 风格 inline SVG**（24×24 / `stroke-width="2"`） |
| **同一组件内两种风格** | `ChatContextBar.vue` 头部用 emoji（`:7,11,16,58`），操作按钮用 inline SVG（`:30-45`） |
| **文本字符当图标** | `SessionSettingsDialog.vue:21`（`×`）、`:29`（`♥`）、`HomedirSwitcher.vue:19`（`×`）、`ChatMainPanel.vue:41`（`⚠`）、`ModelChatPanel.vue:620`（`.banner-icon`） |
| **SVG 尺寸/描边不统一** | `MainLayout.vue` 用 `20×20 stroke-width=2`；`ResourceShell.vue:44-54` 用 `16×16`（**且缺 `stroke-linecap`/`stroke-linejoin`**）；`ChatMainPanel.vue:10,19` 用 `14×14`；`ChatContextBar.vue:30-45` 用 `13×13` → **4 种尺寸** |
| **齿轮图标 path 几何错误（已核实）** | `MainLayout.vue:79`。详见 §3.11-TOP1 |

### 3.10 重复样式与死代码

| 重复项 | 份数 | 位置 | 差异 |
| --- | --- | --- | --- |
| `.toast` | 4 | `AgentView.vue:279`、`McpView.vue:301`、`ModelProvidersView.vue:213`、`SkillView.vue:468` | 底色/尺寸不一致（见 §3.6） |
| `.panel-header` | 3 | `ResourceShell.vue:141`、`SessionListPanel.vue:108`、`SessionExplorerPanel.vue:167` | 基本一致 |
| `.icon-btn` | 8 | `ResourceShell.vue:162`、`ExplorerPage.vue:526`、`FileViewerOverlay.vue:545`、`SessionExplorerPanel.vue:188`、`SessionListPanel.vue:124`、`McpServerSettings.vue:606`、`ModelProvidersSettings.vue:633`、`SettingsPage.vue:449` | 尺寸 24/26/28/32 不一 |
| `.status-dot` | 5 | `ResourceCard.vue:120`、`SessionCard.vue:211`、`McpServerCard.vue:145`、`McpServerSettings.vue:847`、`ModelProviderCard.vue:136` | 尺寸 7px/8px，配色 3 套 |
| `@keyframes pulse` | 8 | `ResourceShell.vue:211`、`ExplorerPage.vue:638`、`SessionCard.vue:238,243`、`SessionListPanel.vue:164`、`McpServerCard.vue:164`、`McpServerSettings.vue:865`、`ModelProviderCard.vue:159` | **3 种不同实现**（opacity 缩放 / box-shadow 扩散环 / 纯 opacity） |
| `.empty-state` | 2（+4 个变体） | `ResourceShell.vue:216`、`SessionListPanel.vue:175` | — |
| 表单样式「同构」 | 2 | `ModelProvidersSettings.vue:486` 注释自述「与 SettingsPage 完全同构」，但为独立 scoped 副本 | 已产生漂移（`.setting-item` padding `0.65rem 0` vs `1rem`） |
| `.menu` 重复定义 | 2（同文件内） | `ChatSettings.vue:526` 与 `:568` | 后者仅追加 `animation` |

**死代码**：

| 文件 | 行数 | 引用情况 |
| --- | --- | --- |
| `components/ExplorerPage.vue` | 873 | **0 引用**（`grep -rn "ExplorerPage" src` 无 import） |
| `components/FloatingInput.vue` | 229 | **0 引用** |
| `components/CodeBlockExecutor.vue` | 211 | **0 引用** |
| `components/Diagnostic.vue` | 107 | **0 引用** |
| **合计** | **1,420 行（占全部 `.vue` 的 9.8%）** | 完全未挂载 |

> 注意：`FileTreeNode.vue` 虽被 `ExplorerPage.vue:162` 引用，但 `ExplorerPage` 本身是死代码；`FileTreeNode` 的**唯一活引用是 `SessionExplorerPanel.vue:64`**，故 `FileTreeNode` 必须改造。

**死 CSS**：`SettingsPage.vue:391-406`（`.active-provider-summary` / `.provider-pill` / `.provider-divider` / `.provider-model`）与 `:438-456`（`.recent-list` / `.recent-item` / `.mcp-servers` / `.mcp-server-card` / `.server-header` / `.server-name` / `.server-actions` / `.server-info` / `.add-server-btn` / `.icon-btn` / `.toggle.small` / `.dialog-overlay` / `.dialog` / `.form-group` / `.checkbox-label` / `.dialog-actions` / `.action-btn.secondary`）→ **约 20 个选择器在模板中无对应元素**，是早期 MCP 设置内联实现的残留。

**其他缺陷**：

| 问题 | 位置 |
| --- | --- |
| 模板中 Unicode 转义未解码，会**原样显示** `Model \u5bf9\u8bdd时` | `SettingsPage.vue:115` |
| 多余空串拼接 `{{ section.label }}{{'' }}` | `SettingsPage.vue:17` |
| 字体栈拼写错误 `-apple-system, BlinkMacMacSystemFont, …`（应为 `BlinkMacSystemFont`） | `MarkdownEditor.vue:289` |
| 占位死链 `<a href="#">` × 3（文档 / GitHub / 反馈） | `SettingsPage.vue:231-233` |
| CSS 注释中的 Unicode 转义 `/* Model \u5bf9\u8bdd侧边栏 */` | `ExplorerPage.vue:808` |
| `props` 变量声明后未使用（潜在 lint / 构建告警） | `ResourceShell.vue:103`、`ResourceCard.vue:63`、`ConfirmDialog.vue:80` |

### 3.11 信息架构与导航评估（详见 §5）

- `views/SessionView.vue:41-56`：三栏 `flex: 0 0 260px` / `flex: 1 1 auto` / `flex: 0 0 280px`。**`flex-shrink: 0` 意味着两侧永不收缩**，窗口 700px 时中间内容区仅剩 160px。无响应式断点、无折叠行为。
- `views/MainLayout.vue`：48px 纯图标侧边栏，6 个导航项**无文字标签、无分组分隔**。

### 3.12 问题清单 Top 10

| # | 问题 | 严重度 | 关键证据 |
| --- | --- | --- | --- |
| **TOP1** | **设置（齿轮）图标 SVG path 几何错误** | P0 | `MainLayout.vue:79`。实测该 path 相对 Feather 标准 `settings` 路径**丢失了两段命令**：<br>① 文件为 `…0-2.83l.06-.06A1.65 1.65 0 0 0 4.6 15a1.65 1.65 0 0 0-1.51-1H3…`，标准应为 `…0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3…`（丢失 `a…33-1.82 1.65`，并凭空插入终点 `4.6 15`）；<br>② 文件为 `…l-.06.06A1.65 1.65 0 0 0 -.33 1.82V9…`，标准应为 `…l-.06.06a1.65 1.65 0 0 0-.33 1.82V9…`（弧参数前缺命令字母、且 `-.33` 前有空格）。<br>**结论**：修正 team-lead 的初始判断 —— 该 path **语法上可解析**（两段 arc 均含 7 个数值参数），**不会导致渲染失败或报错**，但**几何已被破坏**，齿轮会渲染为不对称/变形图形。需整体替换为标准 Feather `settings` path 后目视复核。 |
| **TOP2** | **`:focus-visible` 覆盖率为 0，键盘不可达** | P0 | `grep focus-visible` → 0；`aria-label` 仅 4 处；4 个列表卡片（`ResourceCard` / `SessionCard` / `ModelProviderCard` / `McpServerCard`）为裸 `div + @click`，无 role/tabindex/keydown |
| **TOP3** | **令牌层对比度不达标（4 组）** | P0 | `--color-text-muted` 浅 `#999999` = 2.85:1、深 `#64748b` on `#1e1e2e` = 3.45:1；`--color-primary #667eea` 作填充底/前景字均 3.66:1（`App.vue:19,26,124`） |
| **TOP4** | **深色主题下多处「刺眼白块」** | P0 | `ChatSettings.vue:531`（`background: white` 菜单）、`ModelSelectionDialog.vue:307`（`rgba(255,255,255,0.98)`）、`SettingsPage.vue:458`（`background: white` 对话框）、`ChatContextBar.vue:240,277`（白色代码块与遮罩）、`ChatContextBar.vue:263`（`#1f2937` 深字） |
| **TOP5** | **深色下 hover 完全失效（81 处黑色叠加）** | P0 | `rgba(0, 0, 0, …)` × 81 / 28 文件；典型：`ResourceShell.vue:177`、`SessionListPanel.vue:139`、`MainLayout.vue:249`、`SettingsPage.vue:376,377,387` |
| **TOP6** | **5 个令牌被引用但从未定义 + 4 个令牌仅深色存在** | P0 | 未定义：`--color-border-subtle`、`--font-mono`、`--color-primary-hover`、`--color-text-primary`（无 fallback）、`--border-color`；仅深色：`--color-input-bg` / `--color-surface-strong` / `--color-hover-bg` / `--color-active-bg`（`App.vue:167-170`） |
| **TOP7** | **同类组件视觉语言分裂** | P0 | 选中态 3 派：`ModelProviderCard.vue:113-114` **绿色**左边框 vs `McpServerCard.vue:122-123` / `ResourceCard.vue:97-99` 主色；状态点 5 份实现 3 套配色；图标按钮 5 种尺寸（22/24/26/28/32px）；Toast 4 份实现且默认色不同（`ModelProvidersView.vue:225` 为绿） |
| **TOP8** | **圆角 16 种 / gap 14 种 / padding 30+ 种 / 过渡 7 种，无令牌** | P1 | 见 §3.4 分布表 |
| **TOP9** | **16 处 px 字号不受「小/中/大」档位控制；存在 9px/10px 极小字号** | P1 | `ChatContextBar.vue:161,171,174,204,261,285`、`ModelSelectionDialog.vue:342,347,358,375,381,415`、`ChatInputArea.vue:291`、`MarkdownEditor.vue:289,291,307`；对照 `stores/appearance.ts:27-31,61` |
| **TOP10** | **1,420 行死代码 + 约 20 个死 CSS 选择器；8 处原生 alert/confirm/prompt 未用已有 ConfirmDialog** | P1 | `ExplorerPage.vue`(873) / `FloatingInput.vue`(229) / `CodeBlockExecutor.vue`(211) / `Diagnostic.vue`(107) 零引用；`SettingsPage.vue:391-406,438-456`；原生弹窗见 §3.6 |

---

## 4. 设计系统基准（核心产出）

> **落地方式**：新增 `src/styles/tokens.css`（或扩充 `App.vue` 的非 scoped `<style>`），由 `main.ts` 引入。**保留现有 `--color-*` 名称作为别名映射**，会话区已完成的令牌化工作**零改动、零回退**。

### 4.1 语义令牌清单（浅色 / 深色各一套具体值）

#### 4.1.1 表面 `--surface-*`

| 令牌 | 浅色 | 深色 | 用途 |
| --- | --- | --- | --- |
| `--surface-page` | `#f6f7f9` | `#16161d` | 页面底 / 应用背景（替代 `--color-bg`） |
| `--surface-panel` | `#ffffff` | `#1c1c25` | 侧栏、面板、详情区（替代 `--color-surface`） |
| `--surface-card` | `#ffffff` | `#23232e` | 卡片、列表项抬高态 |
| `--surface-sunken` | `#f0f2f5` | `#101017` | 输入框、代码块、凹陷区（替代 `--color-input-bg`） |
| `--surface-overlay` | `#ffffff` | `#23232e` | 对话框、菜单、Tooltip、Toast（替代 `--color-surface-strong`） |
| `--surface-hover` | `rgba(15,23,42,0.05)` | `rgba(255,255,255,0.06)` | hover 叠加（替代 81 处 `rgba(0,0,0,…)`） |
| `--surface-active` | `rgba(15,23,42,0.09)` | `rgba(255,255,255,0.10)` | pressed / active 叠加 |
| `--surface-selected` | `rgba(79,70,229,0.08)` | `rgba(129,140,248,0.14)` | 列表项选中底色 |

#### 4.1.2 文本 `--text-*`

| 令牌 | 浅色 | 深色 | 说明 |
| --- | --- | --- | --- |
| `--text-primary` | `#181c25` | `#e6e8ee` | 正文 / 标题（浅色对 `--surface-page` ≈ 14.6:1；深色 ≈ 13.8:1） |
| `--text-secondary` | `#4b5563` | `#a2aab8` | 次级说明（浅 7.6:1 / 深 7.2:1） |
| `--text-muted` | `#6b7280` | `#7c8595` | 元信息 / 占位（浅 4.83:1 / 深 4.54:1）——**修正原 `#999999` 2.85:1 与 `#64748b` 3.45:1** |
| `--text-disabled` | `#9aa3b0` | `#5a616f` | 禁用态（WCAG 对 disabled 有豁免） |
| `--text-inverse` | `#ffffff` | `#16161d` | 反色表面上的前景 |
| `--text-on-accent` | `#ffffff` | `#16161d` | 主色填充上的前景 |

#### 4.1.3 描边 `--border-*`

| 令牌 | 浅色 | 深色 | 用途 |
| --- | --- | --- | --- |
| `--border-subtle` | `#eceef2` | `#262632` | hairline 分隔线（补齐未定义的 `--color-border-subtle`） |
| `--border-default` | `#dfe3ea` | `#333343` | 常规描边（替代 `--color-border`） |
| `--border-strong` | `#c6ccd8` | `#454558` | 输入框 hover / 强调描边 |

#### 4.1.4 强调色与语义色 `--accent` / `--success` / `--warning` / `--danger` / `--info`

> **关键决策**：浅色 `--accent` 由 `#667eea` 调整为 `#4f46e5`。理由：原值作填充底（白字）与作前景字均为 **3.66:1**，不达 AA；`#4f46e5` 两项均为 **6.29:1**。深色 `--accent` 保持 `#818cf8`，但**其上的前景改为深色 `--text-on-accent: #16161d`**（白字仅 2.98:1，深色字 6.04:1）。

| 令牌 | 浅色 | 深色 |
| --- | --- | --- |
| `--accent` | `#4f46e5` | `#818cf8` |
| `--accent-hover` | `#4338ca` | `#a5b4fc` |
| `--accent-active` | `#3730a3` | `#c7d2fe` |
| `--accent-subtle-bg` | `rgba(79,70,229,0.10)` | `rgba(129,140,248,0.16)` |
| `--accent-subtle-border` | `rgba(79,70,229,0.30)` | `rgba(129,140,248,0.40)` |
| `--success-fg` / `--success-bg` / `--success-solid` | `#15803d` / `#ecfdf3` / `#16a34a` | `#6ee7a8` / `#10281c` / `#22c55e` |
| `--warning-fg` / `--warning-bg` / `--warning-solid` | `#b45309` / `#fffbeb` / `#f59e0b` | `#fcd34d` / `#2a2109` / `#f59e0b` |
| `--danger-fg` / `--danger-bg` / `--danger-solid` | `#b91c1c` / `#fef2f2` / `#dc2626` | `#fca5a5` / `#2a1618` / `#ef4444` |
| `--info-fg` / `--info-bg` / `--info-solid` | `#1d4ed8` / `#eff6ff` / `#3b82f6` | `#93c5fd` / `#0f2035` / `#3b82f6` |

> **收敛效果**：现有 **红 7 种**（`#ef4444 / #dc2626 / #b91c1c / #dc3545 / #d33 / #fca5a5 / #fee`）、**绿 5 种**（`#22c55e / #16a34a / #15803d / #10b981 / #d4edda`）、**黄 5 种**（`#f59e0b / #b45309 / #fcd34d / #92400e / #d97706`）→ 各收敛为 **3 个令牌（fg / bg / solid）**。

#### 4.1.5 圆角 `--radius-*`

| 令牌 | 值 | 适用 |
| --- | --- | --- |
| `--radius-xs` | `3px` | 极小型徽章 / pill 内嵌 |
| `--radius-sm` | `4px` | 标签、chip、小型控件 |
| `--radius-md` | `6px` | **默认控件**：按钮、输入框、选择器 |
| `--radius-lg` | `8px` | 卡片、列表项、面板 |
| `--radius-xl` | `12px` | 对话框、浮层、大容器 |
| `--radius-full` | `999px` | 圆形头像 / pill / 状态点 |

> **规则**：容器 ≥ 控件；**禁止 12px 以上（除 `--radius-full`）**；禁止非对称多值圆角（`MessageNode.vue:1063` 的 `14px 14px 4px 14px` 属会话气泡设计语言，**作为唯一例外保留，需用户确认**——见 §8）。

#### 4.1.6 间距 `--space-*`（4pt 基准）

| 令牌 | 值 | 典型用途 |
| --- | --- | --- |
| `--space-0` | `0` | — |
| `--space-05` | `0.125rem`（2px） | 图标与文字的极紧间隙（需评审后使用） |
| `--space-1` | `0.25rem`（4px） | 图标按钮间 `gap`、紧凑内边距 |
| `--space-2` | `0.5rem`（8px） | **默认 `gap`**、卡片内边距 |
| `--space-3` | `0.75rem`（12px） | 面板内边距、表单项间距 |
| `--space-4` | `1rem`（16px） | 分区间距、内容区边距 |
| `--space-5` | `1.5rem`（24px） | 大分区间距 |
| `--space-6` | `2rem`（32px） | 页面级留白 |

> **规则**：`gap` 只允许 `--space-1/2/3`；容器 `padding` 只允许 `--space-2/3/4/5`；**禁用裸 `rem`/`px`**。

#### 4.1.7 字体与排版

```css
--font-sans: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'PingFang SC',
             'Microsoft YaHei', Roboto, Helvetica, Arial, sans-serif;
--font-mono: 'JetBrains Mono', 'Fira Code', ui-monospace, SFMono-Regular,
             Menlo, Consolas, monospace;        /* 补齐未定义令牌 */

--font-size-xs:   0.6875rem;  /* 11px —— 下限，禁止更小 */
--font-size-sm:   0.75rem;    /* 12px */
--font-size-base: 0.875rem;   /* 14px —— 正文 */
--font-size-md:   1rem;       /* 16px */
--font-size-lg:   1.125rem;   /* 18px —— 小节标题 */
--font-size-xl:   1.25rem;    /* 20px —— 页面 / 详情标题 */
--font-size-2xl:  1.5rem;     /* 24px —— 仅「关于」页 App 名 */

--font-weight-regular: 400;
--font-weight-medium: 500;
--font-weight-semibold: 600;
--font-weight-bold: 700;      /* 禁止用于正文 */

--line-height-tight: 1.35;    /* 标题 */
--line-height-normal: 1.5;    /* 正文 */
--line-height-relaxed: 1.6;   /* 长文 / 说明 */
```

#### 4.1.8 阴影 / 动效 / 焦点环 / 尺寸

```css
/* 阴影：仅 2 级，克制 */
--shadow-1: 0 1px 2px rgba(16,24,40,0.06);              /* hover 微抬升 */
--shadow-2: 0 8px 24px rgba(16,24,40,0.12);             /* 浮层：对话框/菜单/Tooltip */
/* 深色下阴影不可见，用描边替代 */
:root[data-theme="dark"] {
  --shadow-1: none;                                      /* 改用 border 表达层级 */
  --shadow-2: 0 8px 24px rgba(0,0,0,0.5);
}

/* 动效 */
--motion-fast: 120ms;
--motion-base: 160ms;
--motion-slow: 240ms;
--motion-ease: cubic-bezier(0.2, 0, 0.2, 1);

/* 焦点环 */
--focus-ring-width: 2px;
--focus-ring-offset: 2px;
--focus-ring-color: var(--accent);

/* 控件高度 */
--control-height-sm: 24px;   /* 密集列表内图标按钮（下限，不可更小） */
--control-height-md: 28px;   /* 默认图标按钮 */
--control-height-lg: 32px;   /* 按钮 / 输入框 / 选择器 */
--hit-target-min: 24px;      /* WCAG 2.2 AA 目标尺寸下限 */
```

#### 4.1.9 别名映射层（保证不回退）

```css
:root {
  --color-bg:        var(--surface-page);
  --color-surface:   var(--surface-panel);
  --color-border:    var(--border-default);
  --color-border-subtle: var(--border-subtle);   /* 补齐 */
  --color-text:      var(--text-primary);
  --color-text-secondary: var(--text-secondary);
  --color-text-muted:     var(--text-muted);
  --color-text-primary:   var(--text-primary);   /* 补齐 */
  --color-primary:   var(--accent);
  --color-primary-dark: var(--accent-hover);
  --color-primary-hover:  var(--accent-hover);   /* 补齐 */
  --color-input-bg:      var(--surface-sunken);  /* 补齐浅色 */
  --color-surface-strong: var(--surface-overlay);/* 补齐浅色 */
  --color-hover-bg:      var(--surface-hover);   /* 补齐浅色 */
  --color-active-bg:     var(--surface-active);  /* 补齐浅色 */
  --border-color:        var(--border-default);  /* 补齐 */
}
```

### 4.2 排版与层级规范

| 层级 | 字号 | 字重 | 行高 | 字色 |
| --- | --- | --- | --- | --- |
| H1（页面/详情标题） | `--font-size-xl` (20) | 600 | tight | `--text-primary` |
| H2（分区标题） | `--font-size-lg` (18) | 600 | tight | `--text-primary` |
| H3（面板标题 / 卡片标题） | `--font-size-base`(14) 或 `--font-size-md`(16) | 600 | tight | `--text-primary`（⚠️ 修正 `ResourceShell.vue:153`、`SessionListPanel.vue:120` 当前用 `--color-text-secondary`） |
| 正文 | `--font-size-base` (14) | 400 | normal | `--text-primary` |
| 次级说明 | `--font-size-sm` (12) | 400 | normal | `--text-secondary` |
| 元信息 / 时间戳 / 路径 | `--font-size-sm` (12) | 400 | normal | `--text-muted` |
| 标签 / 徽章 | `--font-size-xs` (11) | 500 | tight | 语义 `-fg` |
| 等宽（代码 / ID / 路径） | `--font-size-sm` (12) | 400 | normal | `--text-primary` + `--font-mono` |

**规则**：
1. 全局基准 `font-size: 0.875rem`（保持 `App.vue:86` 现状，确保「中」档 = 现有观感）。
2. **禁止 `px` 字号**，一律 `rem`，否则「小/中/大」档位失效（`stores/appearance.ts:27-31`）。
3. **禁止小于 11px 的字号**（当前 `ChatContextBar.vue:204` 为 9px、`:174` 为 10px）。
4. **禁止正文使用 `font-weight: 700`**；标题用 600。
5. 中文界面补 `'PingFang SC' / 'Microsoft YaHei'`（当前 `App.vue:82` 字体栈缺中文字体）。

### 4.3 组件视觉基线

| 组件 | 规格 |
| --- | --- |
| **按钮·主** | 高 32px；`padding: 0 var(--space-3)`；`radius: --radius-md`；底 `--accent`，字 `--text-on-accent`；hover → `--accent-hover`；active → `--accent-active`；disabled → `--surface-sunken` + `--text-disabled`（**不用 `opacity: 0.5`**，保证文字对比度可控） |
| **按钮·次** | 底 `transparent`，边 `--border-default`，字 `--text-primary`；hover → `--surface-hover`；active → `--surface-active` |
| **按钮·危险** | 底 `--danger-solid`，字 `#fff`（浅）/ `--text-inverse`（深）；hover → 深一档；**不使用原生 `confirm()`** |
| **按钮·幽灵** | 底 `transparent`，无边，字 `--text-secondary`；hover → `--surface-hover` + `--text-primary` |
| **输入框 / 文本域** | 高 32px（多行自适应，`min-height` 32）；底 `--surface-sunken`，边 `--border-default`，`radius: --radius-md`，字 `--font-size-base`；focus → 边 `--accent` + `outline: none` **且** `--focus-ring`；placeholder `--text-muted`；disabled → 底 `--surface-sunken`，字 `--text-disabled` |
| **卡片 / 列表项** | 无外框（去装饰）；`padding: var(--space-2) var(--space-3)`；`radius: --radius-lg`；默认底 `transparent`；hover → `--surface-hover`；**选中 → `--surface-selected` + 左侧 2px `--accent` 指示条**（统一 3 派选中态）；靠「标题行 / 节点头」分组，不靠边框 |
| **标签 / 徽章** | `padding: 0 var(--space-1)`（高 18px）；`radius: --radius-sm`；字 `--font-size-xs` / 500；底 = 语义 `-bg`，字 = 语义 `-fg`；**禁止 `border-radius: 999px` 与 `3px` 混用**（统一 `--radius-sm`，pill 场景用 `--radius-full`） |
| **对话框 / 浮层** | 底 `--surface-overlay`；`radius: --radius-xl`；`box-shadow: --shadow-2`；**浅色加 `1px --border-default`，深色加 `1px --border-strong`（阴影在深色不可见）**；遮罩统一 `rgba(0,0,0,0.45)`（当前 0.45 / 0.5 混用）；`z-index` 集中管理（当前散落 100 / 1000 / 1500 / 2000 / 9999） |
| **空状态** | 居中；图标 24px `--text-muted`；主文案 `--font-size-base` `--text-secondary`；副文案 `--font-size-sm` `--text-muted`；可选一个次按钮。全 App **1 个 `EmptyState` 组件** |
| **加载 / 骨架** | 短时（<300ms）不显示；列表首载用骨架屏（底 `--surface-sunken`，`radius: --radius-md`，1.4s shimmer）；按钮内用 12px spinner（`border: 2px --border-default; border-top-color: var(--accent)`）。**禁止 `transform: rotate` 之外的装饰动画** |
| **错误提示** | 内联：字 `--font-size-sm` `--danger-fg`，底 `--danger-bg`，`radius: --radius-sm`；整页错误：图标 + 标题 + 描述 + 「重试/忽略」两按钮（`ChatMainPanel.vue:272` 为基线）；Toast 见下 |
| **Toast** | **1 个全局组件**；底 `--surface-overlay` + `1px --border-default`，`radius: --radius-lg`，`shadow-2`；左侧 3px 语义色条（success/warning/danger/info）；字 `--font-size-sm`；3s 自动消失；可点击关闭；`role="status"` + `aria-live="polite"` |
| **Tooltip** | 底 `--surface-overlay`，`radius: --radius-sm`，`shadow-1`，字 `--font-size-xs`，`padding: var(--space-1) var(--space-2)`；延迟 300ms |
| **开关 / 复选框** | 开关 40×22（会话设置）/ 48×24（设置页，二选一，**建议统一 40×22**）；关态底 `--border-strong`，开态底 `--accent`；滑块 `#fff`（浅）/ `--surface-overlay`（深）；**必须有 `:focus-visible` 焦点环**（当前 `SettingsPage.vue:410`、`SessionSettingsDialog.vue:337` 均为 `opacity:0` 不可见） |

### 4.4 交互规范

| 态 | 规则 |
| --- | --- |
| `hover` | 背景叠加 `--surface-hover`（**禁止 `rgba(0,0,0,0.0x)`**）；过渡 `--motion-fast` |
| `active` | 背景叠加 `--surface-active`；**禁止装饰性 `transform: scale()`** |
| `focus-visible` | `outline: var(--focus-ring-width) solid var(--focus-ring-color); outline-offset: var(--focus-ring-offset);`。**全局统一，禁止 `outline: none` 无替代** |
| `disabled` | 底 `--surface-sunken`，字 `--text-disabled`，`cursor: not-allowed`；**优先用色值而非 `opacity`** |
| `selected` | `--surface-selected` + 2px `--accent` 指示条 + `aria-selected="true"` |

**过渡**：仅允许 `background-color / border-color / color / opacity / box-shadow`；时长只取 `--motion-fast | --motion-base | --motion-slow`；缓动统一 `--motion-ease`。

**最小可点击区域**：所有交互元素 **≥ 24×24px**（WCAG 2.2 AA）。当前 22px 的 3 处（`SessionCard.vue:337-338`、`ChatContextBar.vue:221-222`、`ModelSelectionDialog.vue:353-354`）需提升到 ≥24px。

### 4.5 无障碍

| 项 | 要求 | 验收 |
| --- | --- | --- |
| 正文对比度 | ≥ 4.5:1 | 见 §7.3 组合清单 |
| 大字（≥18.66px 粗体 或 ≥24px） | ≥ 3:1 | 同上 |
| UI 组件/图形对比度 | ≥ 3:1（边框、图标、状态点） | 同上 |
| 焦点可见 | 所有可聚焦元素 `:focus-visible` 有 2px 焦点环 | 100% 覆盖 |
| 键盘可达 | Tab 可遍历全部交互元素；卡片 `role="button"` + `tabindex="0"` + `Enter/Space`；对话框有焦点陷阱 + `Esc` 关闭 | 见 §7.4 |
| 读屏 | 纯图标按钮 100% 有 `aria-label` 或 `title`；列表容器 `role="listbox"` + 项 `role="option"` + `aria-selected`；对话框 `role="dialog|alertdialog"` + `aria-modal="true"` + `aria-labelledby` | 见 §7.5 |
| 动效 | 支持 `@media (prefers-reduced-motion: reduce)` 关闭非必要动画 | P2 |

### 4.6 「去装饰」原则（硬性）

1. **禁止装饰性渐变**：`linear-gradient` / `radial-gradient` 只允许用于**内容渐隐遮罩**（`mask-image` 或 `to bottom, transparent, <surface>`），且颜色必须引用 `--surface-*` 令牌。→ 需清理 6 处（§3.3）。
2. **禁止多重描边**：不出现「`box-shadow` + `0 0 0 1px rgba(0,0,0,0.05)`」的描边二重奏，一律改用 `1px solid var(--border-*)`。→ 需清理 4 处。
3. **禁止光晕 / 扩散环**：状态点不使用 `box-shadow` 发光或脉冲环，统一 `opacity` 呼吸。→ 需清理 3 处。
4. **色彩只承载语义**：主色 / 成功 / 警示 / 危险 / 信息。任何纯装饰性色块、色条、彩色渐变一律移除。
5. **阴影只 2 级**：`--shadow-1`（hover 微抬升）、`--shadow-2`（浮层）。其余删除。
6. **扁平优先**：层级靠「表面色阶 + hairline 分隔线」表达，**不靠边框包裹**。内容分组靠「节点头 / 标题」，不靠框。

---

## 5. 信息架构与导航评估

### 5.1 48px 纯图标侧边栏

| 评估项 | 现状 | 判断 | 建议 |
| --- | --- | --- | --- |
| 图标可辨识度 | 6 个图标：会话（对话气泡）/ Model Provider（服务器+双点）/ Agent（人像）/ MCP（四宫格）/ Skill（星形）/ 设置（齿轮，**已损坏**） | ⚠️ **语义区分度不足**：Model Provider 与 MCP 均为「方块阵列」类图标，初次使用难以区分；Agent 与设置的人像/齿轮在非标准几何下易混 | 建议：① 先修齿轮 path；② Provider 改用「云 + 插头」或「钥匙」语义；MCP 保留四宫格但加连接线强化 |
| 无文字标签 | 全部依赖 `title` tooltip | ⚠️ 对首次用户不友好，且 tooltip 触发有 300ms+ 延迟 | **建议加文字标签**（侧栏 48px → 72px 或 176px），或使用「图标 + 12px 文字」的紧凑双行布局。**需用户拍板**（§8-Q2） |
| 分组 | 6 项平铺，无分隔线；底部「系统目录」用 `border-top`（`MainLayout.vue:230`）与上方分隔 | ⚠️ 中间 5 项（Provider / Agent / MCP / Skill）同属「资源配置」却与「会话」同级平铺 | 建议：会话单独一组；Provider / Agent / MCP / Skill 归入「资源」组并加分组标题（如「资源」）；设置 + 系统目录置于底部。同样**需用户拍板** |
| 顺序 | 会话 → Model Provider → Agent → MCP → Skill → 设置 | 基本合理（会话为首要任务） | 可微调：会话 → Agent → Skill → MCP → Model Provider → 设置（按使用频次与「从抽象到具体」）。**需用户拍板** |
| 选中态 | `background: rgba(102,126,234,0.15)` + 主色图标 + `opacity: 1`（`MainLayout.vue:253-257`）；默认 `opacity: 0.65`（`:244`） | ⚠️ 用 `opacity: 0.65` 压暗未选中项会降低对比度（`#666 × 0.65` on `#fff` ≈ 3.2:1） | 改为：未选中 `--text-secondary`，选中 `--text-primary` + `--surface-selected` + 主色；删除 `opacity` 手法 |

### 5.2 三栏布局在窄窗口下的表现

**现状**（`views/SessionView.vue:41-56`）：

```css
.col-left  { flex: 0 0 260px; min-width: 200px; max-width: 360px; }
.col-middle{ flex: 1 1 auto;  min-width: 0; }
.col-right { flex: 0 0 280px; min-width: 200px; max-width: 420px; }
```

**问题**：两侧 `flex-shrink: 0` + `flex-basis` 固定 → **两侧永不收缩**（`min-width`/`max-width` 实际不生效）。

| 窗口宽度 | 中间内容区实际宽度 | 可用性 |
| --- | --- | --- |
| 1440px | 900px | ✅ |
| 1100px | 560px | ✅ |
| 900px | 360px | ⚠️ 拥挤 |
| 700px | 160px | ❌ 不可用 |
| 600px | 60px | ❌ 崩溃 |

**建议**（按优先级）：

| 方案 | 说明 | 代价 |
| --- | --- | --- |
| A. 响应式断点 | `@media (max-width: 1100px)`：右栏（文件树）自动收起为 48px 图标条或可折叠；`@media (max-width: 900px)`：左栏（会话列表）改为抽屉式覆盖 | 低 |
| B. 可拖拽分栏 + 记忆宽度 | 复用 `ExplorerPage.vue:859` 的 `.chat-resize-handle` 思路；宽度写入 localStorage | 中 |
| C. 手动折叠开关 | 左右栏各加一个折叠按钮（`ChatSettings` 已有 `compact` 思路可复用） | 低 |

> 建议 **A + C 组合**，B 列为 P2。

### 5.3 子页面分组与顺序

| 现状顺序 | 建议顺序 | 理由 |
| --- | --- | --- |
| 会话 / Model Provider / Agent / MCP / Skill / 设置 | 会话 → 〔资源组：Agent / Skill / MCP / Model Provider〕→ 设置 | Agent / Skill / MCP / Provider 同属「可配置资源」，且四者已共享 `ResourceShell` 骨架，视觉与心智模型一致；会话是主任务独占一组 |

> ⚠️ **调整顺序与分组属于信息架构变更，需用户拍板（§8-Q1）。若用户不同意，则保持现状，仅做视觉令牌化。**

---

## 6. 需求池

> 优先级定义：**P0 = 不改动就有明显体验/一致性问题**；P1 = 明显改善但可延后；P2 = 锦上添花。

### ① 设计系统与令牌底座

| ID | 优先级 | 需求 | 涉及模块与文件 | 验收标准 |
| --- | --- | --- | --- | --- |
| R01 | **P0** | 建立统一语义令牌层（§4.1 全部 9 组），浅色/深色各一套 | 新增 `src/styles/tokens.css`（或扩充 `App.vue:18-171`），`main.ts` 引入 | `tokens.css` 中存在 §4.1 全部令牌且浅/深各一份；`grep` 现有 `--color-*` 均能解析到别名 |
| R02 | **P0** | 补齐 4 个仅深色存在的令牌的浅色定义 | `App.vue:167-170`；消费方 `ChatSettings.vue:550,551,601,612`、`ChatInputArea.vue:217,226` | 浅色下 `--color-input-bg/-surface-strong/-hover-bg/-active-bg` 有真实值；移除这 6 处的 fallback 后视觉无变化 |
| R03 | **P0** | 补齐 5 个「被引用但未定义」的令牌 | `--color-border-subtle`(6 处)、`--font-mono`(8 处)、`--color-primary-hover`(`ModelProvidersSettings.vue:706`)、`--color-text-primary`(`HomedirSwitcher.vue:239,275,306`)、`--border-color`(`ExplorerPage.vue:655,744`) | `grep -rhoP 'var\(\s*--[a-zA-Z0-9-]+' \| sort -u` 与已定义集合的差集 = **空** |
| R04 | **P0** | 修正对比度不达标的令牌值 | `App.vue:19,26,124`；受影响：`#999`→`#6b7280`、`#64748b`→`#7c8595`、`#667eea`→`#4f46e5` | §7.3 全部组合 ≥ 4.5:1；C1/C2/C3/C4 全部通过 |
| R05 | **P0** | 新增全局基础样式：字体栈补中文、`focus-visible` 全局焦点环、滚动条令牌化 | 新增 `src/styles/base.css` 或扩充 `App.vue:76-114`；`App.vue:108,113` | Tab 遍历任意页面焦点可见；深色下滚动条与背景协调；中文在 Windows/macOS 均正确回退 |
| R06 | P1 | 删除 3 个死令牌（`--header-height`、`--color-msg-card-border`、`--card-pad-y`）；统一 `--msg-gap` 单一定义源 | `App.vue:30,37,38,129,130`；`ModelChatPanel.vue:556` | `--msg-gap` 仅 1 处定义；死令牌移除后构建无引用残留 |
| R07 | P2 | 令牌自动化校验（CI / 本地脚本）：检测硬编码色值、未定义令牌 | 新增 `scripts/check-tokens.mjs` | 脚本可运行，硬编码色值 > 0 时非零退出 |

### ② 外壳与导航

| ID | 优先级 | 需求 | 涉及模块与文件 | 验收标准 |
| --- | --- | --- | --- | --- |
| R08 | **P0** | 修复设置（齿轮）图标 SVG path 几何错误（TOP1） | `MainLayout.vue:79` | 替换为标准 Feather `settings` path；浅/深双主题下目视齿轮对称、齿数均匀；与其余 5 个图标视觉重量一致 |
| R09 | **P0** | 侧边栏导航态令牌化 + 去 `opacity` 手法 + 去装饰 | `MainLayout.vue:209,210,244,249,253-257` | hover / active 使用 `--surface-hover` / `--surface-selected`；深色下 hover 可见；未选中项对比度 ≥ 4.5:1 |
| R10 | **P0** | 侧边栏 6 个图标按钮补 `aria-label`（当前仅底部有） | `MainLayout.vue:8-81` | 纯图标按钮 `aria-label` 覆盖率 100%；与 `title` 文案一致 |
| R11 | P1 | 侧边栏信息架构调整（分组 + 顺序 + 是否加文字标签） | `MainLayout.vue:3-97` | **待用户拍板（§8-Q1/Q2）后执行**；若拍板加标签，则侧栏宽度与布局同步调整且窄窗口下不溢出 |
| R12 | P1 | 图标语义优化（Provider / MCP 区分度） | `MainLayout.vue:24-31, 50-59` | 5 人盲测能正确指出 Provider 与 MCP 图标（≥4/5） |
| R13 | P1 | 统一 SVG 图标尺寸与描边规格 | `MainLayout.vue`(20×20)、`ResourceShell.vue:44-54`(16×16，缺 linecap)、`ChatMainPanel.vue:10,19`(14×14)、`ChatContextBar.vue:30-45`(13×13) | 统一为 16px（控件内）/ 20px（导航）两档；全部带 `stroke-linecap="round" stroke-linejoin="round"` |

### ③ 会话区收尾（衔接已完成改造，禁止回退）

| ID | 优先级 | 需求 | 涉及模块与文件 | 验收标准 |
| --- | --- | --- | --- | --- |
| R14 | **P0** | ChatContextBar 去硬编码浅色 + 去装饰性渐变 | `chat/ChatContextBar.vue:127,138,142,175,205,240,263,277` | 深色下：卡片底为令牌、代码预览块非白、正文 `#1f2937` 已替换、渐隐遮罩颜色随主题；装饰性渐变 2 处移除（渐隐遮罩保留） |
| R15 | **P0** | ChatSettings 下拉菜单深色适配 + 消除重复定义 + 去双重阴影 | `chat/ChatSettings.vue:487,526-570(重复),531,533,550,551,601,610,612` | 深色下菜单为 `--surface-overlay`；`.menu` 仅 1 处定义；`box-shadow` 单一 + `border` |
| R16 | **P0** | 会话区 px 字号改 rem（使「小/中/大」生效） | `chat/ChatContextBar.vue:161,171,174,204,261,285`、`chat/ChatInputArea.vue:291` | 切换「小/中/大」档位时这些元素字号随之变化；无 <11px 字号 |
| R17 | **P0** | 会话区残余硬编码语义色令牌化 | `MessageNode.vue:1011,1035,1241,1306`、`session/ChatMainPanel.vue:200,223,328,333`、`session/SessionCard.vue:220,226,231,235,222`、`chat/ChatInputArea.vue:257,261,264,289-290` | 全部替换为 `--success-*/--danger-*/--text-*/--surface-*`；视觉无回退 |
| R18 | P1 | SessionCard 三态装饰性横向渐变改扁平 | `session/SessionCard.vue:187,191,195` | 改为 `background` + 左侧 2px 语义色条；深色下正确 |
| R19 | P1 | 会话气泡非对称圆角保留或对齐令牌（**待确认**） | `MessageNode.vue:1063`（`14px 14px 4px 14px`） | 见 §8-Q5 |
| R20 | P2 | 会话区骨架屏替代纯文案加载态 | `session/ChatMainPanel.vue:52-55,260-267` | 首载显示 3 条骨架消息 |

### ④ 设置页与四个资源页

| ID | 优先级 | 需求 | 涉及模块与文件 | 验收标准 |
| --- | --- | --- | --- | --- |
| R21 | **P0** | SettingsPage 深色致命项全量令牌化 | `SettingsPage.vue:376,377,384-385,387,398-399,411-412,415,417,420,427,439,447,450-451,458,467` | 深色下：导航 hover/active 非亮灰、消息条可读、开关轨道非亮灰、对话框非纯白 |
| R22 | **P0** | 修复 SettingsPage 文案 bug：`Model \u5bf9\u8bdd时` 字面量（`:115`）、`{{'' }}` 多余拼接（`:17`） | `SettingsPage.vue:17,115` | 界面显示正常中文「Model 对话时…」；无多余空串 |
| R23 | **P0** | 四个资源页「选中态」统一 | `settings/ModelProviderCard.vue:113-114`（绿）vs `settings/McpServerCard.vue:122-123`、`common/ResourceCard.vue:97-99`（主色） | 三个组件选中态均为 `--surface-selected` + 左侧 2px `--accent`；相邻页面切换时选中色一致 |
| R24 | **P0** | 统一状态点（5 份实现） | `common/ResourceCard.vue:120-132`、`session/SessionCard.vue:211-235`、`settings/McpServerCard.vue:145-161`、`settings/McpServerSettings.vue:847`、`settings/ModelProviderCard.vue:136-157` | 尺寸统一 8px；配色统一为 `--success-solid / --warning-solid / --danger-solid / --border-strong`；脉冲统一为 `opacity` 呼吸（1 份 `@keyframes`） |
| R25 | **P0** | 修复 McpServerCard「failed」与 idle 同色 | `settings/McpServerCard.vue:160-161`（`#94a3b8`）vs `:150`（`#94a3b8`） | 三态（可用 / 已停用 / 错误）视觉可区分 |
| R26 | **P0** | Toast 收敛为 1 个全局组件 | 新增 `components/common/Toast.vue`；替换 `AgentView.vue:279`、`McpView.vue:301`、`ModelProvidersView.vue:213`、`SkillView.vue:468` | `.toast` 定义仅 1 处；四页 Toast 默认底色/尺寸/动效一致 |
| R27 | P1 | SettingsPage 清理约 20 个死 CSS 选择器 | `SettingsPage.vue:391-406,438-456` | 每个选择器在模板中均有对应元素（或已删除） |
| R28 | P1 | 表单样式「同构」抽取为共享样式 | `SettingsPage.vue:407,429,460-467,486` 与 `settings/ModelProvidersSettings.vue:497-547,604-612`、`settings/McpServerSettings.vue:785` | `.setting-item / .setting-info / .setting-desc / .action-btn` 单一定义源，两处视觉一致 |
| R29 | P1 | 统一卡片/列表项视觉基线（3 种语汇 → 1 种） | `common/ResourceCard.vue:81-104`（外框+圆角+margin）、`session/SessionCard.vue:165-196`（无框+左指示条）、`settings/ModelProviderCard.vue:98-119` / `settings/McpServerCard.vue:107-128`（底部 hairline） | 四者采用 §4.3「卡片/列表项」统一基线；视觉并排对比无差异 |
| R30 | P1 | 开关控件统一 + 焦点可见 | `SettingsPage.vue:409-414,454-456`、`session/SessionSettingsDialog.vue:330-358` | 尺寸统一 40×22；深浅色正确；`:focus-visible` 有可见焦点环 |
| R31 | P1 | 语义色收敛（红 7 种 / 绿 5 种 / 黄 5 种 → 各 3 令牌） | 全量（见 §3.2 各表） | `grep` 红色系只剩 `--danger-*`；绿色系只剩 `--success-*`；黄色系只剩 `--warning-*` |
| R32 | P2 | SettingsPage 死链处理 | `SettingsPage.vue:231-233` | 三个链接指向真实地址或移除 |

### ⑤ 公共组件与弹层

| ID | 优先级 | 需求 | 涉及模块与文件 | 验收标准 |
| --- | --- | --- | --- | --- |
| R33 | **P0** | 列表卡片键盘可达性 | `common/ResourceCard.vue:12-16`、`session/SessionCard.vue`、`settings/ModelProviderCard.vue:92-94`、`settings/McpServerCard.vue:101-103` | 均有 `role="option"`（容器内 `role="listbox"`）+ `tabindex="0"` + `@keydown.enter/space` + `aria-selected`；仅键盘可完成切换会话/选择 Provider/选择 MCP Server |
| R34 | **P0** | 弹层底色与遮罩统一 | `common/ConfirmDialog.vue:143,205,134`、`chat/ChatSettings.vue:601,600`、`SettingsPage.vue:457-458`、`common/HomedirSwitcher.vue:212,204`、`session/SessionSettingsDialog.vue:240,230`、`ModelSelectionDialog.vue:307` | 全部使用 `--surface-overlay` + `--shadow-2` + `1px --border-*`；遮罩统一 `rgba(0,0,0,0.45)`；`z-index` 集中常量 |
| R35 | **P0** | ModelSelectionDialog 全量令牌化（当前 8 处硬编码，深色下近乎纯白） | `ModelSelectionDialog.vue:307,309,314,327,337,349,359,364,376,382,384,407,412,426,438,444` | 深色下对话框为深色表面；删除失效的 `backdrop-filter`；px 字号改 rem |
| R36 | **P0** | 全局 `hover/active` 叠加令牌化（81 处 `rgba(0,0,0,…)`） | 28 个文件（见 §3.2/§3.4） | 深色下所有 hover 可见；`grep 'rgba(0, *0, *0'` 在样式中仅剩遮罩与阴影定义 |
| R37 | P1 | 空 / 加载 / 错误态组件化 | 新增 `common/EmptyState.vue`、`common/ErrorState.vue`、`common/Skeleton.vue`；替换 6 处空状态、4 处错误态 | 全 App 空状态视觉一致（图标+主文案+副文案+可选操作） |
| R38 | P1 | 原生 `alert/confirm/prompt` 替换为 `ConfirmDialog` / 应用内 Prompt | `session/SessionListPanel.vue:84,87,92`、`session/ChatMainPanel.vue:143,151`、`views/AgentView.vue:146`、`views/ModelProvidersView.vue:176`、`fileViewer/FileViewerOverlay.vue:352`、`chat/ChatSettings.vue:440` | 8 处全部替换；无浏览器原生弹窗 |
| R39 | P1 | `ConfirmDialog` 补齐焦点陷阱（文档已声明但未实现） | `common/ConfirmDialog.vue:8,16,119-127` | 打开后焦点进入对话框；Tab 循环不逃逸；`Esc` 关闭；关闭后焦点归还触发元素 |
| R40 | P1 | 抽取共享样式：`.panel-header`(3) / `.icon-btn`(8) / `.status-dot`(5) / `@keyframes pulse`(8) / `.empty-state`(2) | 见 §3.10 表格 | 每类仅 1 份定义；`.icon-btn` 尺寸收敛为 24 / 28 / 32 三档 |
| R41 | P2 | 最小点击区域提升到 ≥24px | `session/SessionCard.vue:337-338`(22)、`chat/ChatContextBar.vue:221-222`(22)、`ModelSelectionDialog.vue:353-354`(22) | 全部 ≥24×24 |

### ⑥ 编辑器与文件查看器

| ID | 优先级 | 需求 | 涉及模块与文件 | 验收标准 |
| --- | --- | --- | --- | --- |
| R42 | **P0** | FileTreeNode hover/选中令牌化（唯一活引用在 `SessionExplorerPanel.vue:64`） | `FileTreeNode.vue:166,170` | 深色下树节点 hover/选中可见；与会话区其他列表 hover 一致 |
| R43 | **P0** | CodeEditor 编辑器配色令牌化（当前浅色主题下仍为深色） | `CodeEditor.vue:174,175,183,194,195,208,212` | 使用 `--color-code-bg/fg` 或 `--surface-sunken` 派生；行号对比度 ≥ 4.5:1；浅色主题下编辑器为浅色（**待确认 Q6**） |
| R44 | P1 | 圆角收敛到 `--radius-*`（16 种 → 5 档） | 全量（见 §3.4） | `grep -rhoP 'border-radius: *[^;]+'` 的取值集合 ⊆ {3,4,6,8,12,999,50%} 且全部经令牌引用 |
| R45 | P1 | 间距收敛到 `--space-*`（gap 14 种 → 3 档；padding 规范化） | 全量 | `gap` 取值 ⊆ {`var(--space-1)`, `--space-2`, `--space-3`} |
| R46 | P1 | 过渡时长收敛（7 种 → 3 档） | 全量 | 时长 ⊆ {`--motion-fast`, `--motion-base`, `--motion-slow`} |
| R47 | P1 | ExplorerPage 硬编码清理（若决定保留） | `ExplorerPage.vue:540,559,595,618,631,655,704,744,751-752,844` | 同 R36/R42 标准。**若 Q4 决定删除则本项作废** |
| R48 | P1 | MarkdownEditor 修正字体栈 typo + 硬编码 | `MarkdownEditor.vue:286,289,291,300,307` | `BlinkMacSystemFont` 拼写正确；浮动提示与工具栏底色令牌化 |
| R49 | P1 | 三栏布局响应式（方案 A + C） | `views/SessionView.vue:41-56` | 1100px 时右栏可折叠；900px 时左栏抽屉化；700px 时内容区 ≥ 400px |
| R50 | P2 | 三栏可拖拽分栏 + 宽度记忆（方案 B） | `views/SessionView.vue` | 可拖拽，宽度持久化到 localStorage |
| R51 | P2 | 文件查看器独立窗口视觉打磨 | `views/FileViewerWindow.vue:143-236`、`fileViewer/FileViewerOverlay.vue:488,502,512,521,542-543,553,560,597-616,668,676,746` | 与 §4.3 基线一致；圆角/语义色收敛 |

### ⑦ 图标与死代码

| ID | 优先级 | 需求 | 涉及模块与文件 | 验收标准 |
| --- | --- | --- | --- | --- |
| R52 | P1 | 图标风格统一：emoji（140 处 / 19 文件）→ inline SVG | `ExplorerPage.vue`(42)、`FileTreeNode.vue`(40)、`MessageNode.vue`(13)、`ChatSettings.vue`(12)、`SettingsPage.vue`(5)、`McpView.vue`(4)、`ChatContextBar.vue`(4)、`WorkdirPicker.vue`(3)、`ModelSelectionDialog.vue`(3)、`FileViewerOverlay.vue`(3)、`SkillView.vue`(2) 等 | **待用户拍板（§8-Q3）**。若执行：emoji 在 UI 中 0 处；SVG 尺寸/描边统一（同 R13） |
| R53 | P1 | 死代码清理：4 个零引用组件（1,420 行） | `components/ExplorerPage.vue`(873)、`components/FloatingInput.vue`(229)、`components/CodeBlockExecutor.vue`(211)、`components/Diagnostic.vue`(107) | **待用户拍板（§8-Q4）**。若执行：文件删除后 `npm run build` 通过、无残留 import |
| R54 | P2 | `prefers-reduced-motion` 支持 | 全局 | 系统开启「减少动效」后非必要动画停止 |
| R55 | P2 | 组件视觉基线走查页（`/design-review` 路由，仅 dev） | 新增 | 一页展示全部按钮/输入/卡片/标签/对话框/空态/错误态的深浅两主题 |

**统计：P0 = 24 条，P1 = 23 条，P2 = 8 条，合计 55 条。**

> P0 分布：① 令牌底座 5 条 / ② 外壳导航 3 条 / ③ 会话区 4 条 / ④ 设置页与资源页 6 条 / ⑤ 公共组件与弹层 4 条 / ⑥ 编辑器与文件查看器 2 条。
> P0 判定依据：**不改动就会持续存在「功能失效（深色下 hover 不可见 / 白块 / 文字不可读）」「界面破损（齿轮图标变形 / 文案乱码）」「任务阻断（键盘无法选择会话与资源）」三类问题之一的项。**
> 纯一致性打磨（圆角/间距/动效收敛、图标重做、死代码清理）一律归入 P1，避免 P0 膨胀。

---

## 7. 验收标准

### 7.1 构建与类型

| # | 标准 | 命令 |
| --- | --- | --- |
| A1 | 前端构建通过 | `cd D:\Bing\symbio\tauri && npm run build`（= `vite build`）**零错误** |
| A2 | 类型检查无新增错误 | `npx vue-tsc --noEmit` 与改造前基线对比，**无新增 error** |
| A3 | 无新增构建警告 | `npm run build` 输出中无新增 `warning` |
| A4 | 单元测试通过 | `npm test`（`vitest run`）**无回归** |

### 7.2 硬编码与令牌

| # | 标准 | 校验方式 |
| --- | --- | --- |
| B1 | 硬编码色值残留 = 0 | `grep -rhoP '#[0-9a-fA-F]{3,6}\b' src --include=*.vue --exclude=App.vue \| wc -l` = **0**。<br>**白名单（允许残留）**：① `tokens.css` / `App.vue` 令牌定义块内；② 代码块语法高亮色（`MessageNode.vue:1150-1162`）；③ 品牌 logo 相关。白名单需在文档中逐条列出 |
| B2 | 未定义令牌 = 0 | 「已引用令牌集合 − 已定义令牌集合」= **空集**（当前差集含 5 项） |
| B3 | 深色专用令牌浅色补齐 | `--color-input-bg / --color-surface-strong / --color-hover-bg / --color-active-bg` 在 `:root` 与 `:root[data-theme="dark"]` 中**均有定义** |
| B4 | 黑色叠加收敛 | 样式中的 `rgba(0, 0, 0, …)` 仅出现在「遮罩」与「阴影」定义处（当前 81 处） |
| B5 | 圆角取值收敛 | `border-radius` 取值集合 ⊆ {`--radius-xs/sm/md/lg/xl/full`}（当前 16 种） |
| B6 | 单位收敛 | `font-size` 使用 `px` 的处数 = **0**（当前 16 处）；`gap` / `padding` 全部经 `--space-*` 或经评审的极值 |
| B7 | 过渡时长收敛 | 过渡时长集合 ⊆ {`--motion-fast`, `--motion-base`, `--motion-slow`}（当前 7 种） |

### 7.3 对比度（WCAG 2.1 AA）

| # | 前景 / 背景 组合 | 要求 |
| --- | --- | --- |
| C1 | `--text-primary` on `--surface-page` / `--surface-panel` / `--surface-card` / `--surface-overlay` | ≥ 4.5:1（浅/深各 4 组） |
| C2 | `--text-secondary` on 同上 4 种表面 | ≥ 4.5:1（浅/深各 4 组） |
| C3 | `--text-muted` on 同上 4 种表面 | ≥ 4.5:1（浅/深各 4 组） |
| C4 | `--text-on-accent` on `--accent` / `--accent-hover` | ≥ 4.5:1（浅/深各 2 组） |
| C5 | `--success-fg` on `--success-bg`；`--warning-fg` on `--warning-bg`；`--danger-fg` on `--danger-bg`；`--info-fg` on `--info-bg` | ≥ 4.5:1（浅/深各 4 组） |
| C6 | `--accent`（作前景字）on `--surface-panel` | ≥ 4.5:1（浅/深各 1 组） |
| C7 | 边框 / 图标 / 状态点 对相邻色 | ≥ 3:1 |
| C8 | 已知失败项必须转为通过 | C1(2.85) / C2(3.45) / C3(3.66) / C4(3.66) —— 见 §3.5 |

### 7.4 交互态与键盘可达性

| # | 标准 | 校验方式 |
| --- | --- | --- |
| D1 | `:focus-visible` 覆盖率 = 100% | 每个可聚焦元素（按钮 / 输入框 / 链接 / 卡片 / 图标按钮 / 开关 / 导航项）均有 2px 焦点环。**当前为 0%** |
| D2 | 四态齐全 | 抽样 20 个交互元素，均具备 hover / active / focus-visible / disabled（如适用） |
| D3 | 键盘可完成主要任务 | 仅用键盘完成：① 切换会话 ② 发送消息 ③ 新建/选择/删除 Provider ④ 新建/选择/删除 MCP Server ⑤ 切换主题 ⑥ 打开并关闭每个对话框 |
| D4 | 对话框焦点管理 | 打开后焦点进入；Tab 不逃逸；`Esc` 关闭；关闭后焦点归还触发元素。**适用于全部 6 个弹层**：`ConfirmDialog`、`HomedirSwitcher`、`SessionSettingsDialog`、`ChatSettings` modal、`SettingsPage` dialog、`ModelSelectionDialog` |
| D5 | 最小点击区域 ≥ 24×24px | 全部交互元素（当前 3 处为 22px） |
| D6 | 无原生浏览器弹窗 | `grep` 原生 `alert(` / `confirm(` / `prompt(` = **0**（当前 8 处） |

### 7.5 ARIA 与语义

| # | 标准 | 校验方式 |
| --- | --- | --- |
| E1 | 纯图标按钮 100% 有可读名称 | `aria-label` 或 `title` 覆盖率 = 100%（当前仅 4 处 `aria-label`） |
| E2 | 列表语义正确 | 列表容器 `role="listbox"`，项 `role="option"` + `aria-selected`；共 4 个列表（会话 / Provider / MCP / Skill / Agent） |
| E3 | 对话框语义正确 | `role="dialog|alertdialog"` + `aria-modal="true"` + `aria-labelledby`（当前仅 `ConfirmDialog`、`HomedirSwitcher`、`FileViewerOverlay` 有部分） |
| E4 | Toast / 状态提示可播报 | `role="status"` + `aria-live="polite"` |
| E5 | 表单标签关联 | 每个输入控件有 `<label for>` 或 `aria-label`（当前 `SettingsPage.vue` 大量 `.setting-item` 无 `for` 关联） |

### 7.6 视觉一致性（截图核对清单）

**14 张必核截图**（浅色 / 深色 × 7 个页面）：

| # | 页面 / 场景 | 路径 | 重点核对 |
| --- | --- | --- | --- |
| S1/S2 | 会话页（三栏，含消息流与输入区） | `/` | hover/选中态、代码块、ContextBar、输入区 |
| S3/S4 | 设置页·外观 | `/settings` | 导航 hover/active、分段控件、开关、预览卡 |
| S5/S6 | 设置页·会话 / 本地 / 网络 | `/settings` | 输入框、必填标记、消息条 |
| S7/S8 | Model Provider（列表 + 详情表单） | `/model-providers` | 选中态、状态点、表单密度、Toast |
| S9/S10 | MCP Server（列表 + 详情 + 测试结果卡） | `/mcp` | 选中态、测试结果卡、ConfirmDialog |
| S11/S12 | Agent / Skill（列表 + 详情） | `/agent`、`/skill` | 空状态、徽章、代码预览 |
| S13/S14 | 文件查看器（独立窗口） | `/file-viewer` | 编辑器配色、树节点 hover/选中 |

**核对要点**：无白块 / 无不可见 hover / 无低对比文本 / 无装饰性渐变 / 同类元素表现一致 / 焦点环可见。

---

## 8. 待确认问题（需用户拍板）

| ID | 问题 | 选项 | 影响 | 建议 |
| --- | --- | --- | --- | --- |
| **Q1** | 是否允许调整信息架构与导航文案？（侧边栏分组与顺序：会话 → 〔Agent / Skill / MCP / Model Provider〕→ 设置） | A. 允许调整分组与顺序<br>B. 仅允许调整顺序<br>C. 保持现状，只做视觉改造 | 影响 R11；C 选项下 R11 降级为「仅令牌化」 | 建议 **A**（四资源页已共享 `ResourceShell`，分组符合心智模型），但尊重用户选择 |
| **Q2** | 48px 纯图标侧边栏是否加文字标签？ | A. 保持纯图标 48px<br>B. 图标 + 12px 文字，宽度 → 72px<br>C. 图标 + 文字，宽度 → 176px（类 VS Code 活动栏可展开） | 影响全部页面的可用宽度；B/C 下中间内容区减少 24~128px | 建议 **B**（折中：可辨识且不牺牲过多宽度） |
| **Q3** | 是否统一重做图标（140 处 emoji → inline SVG）？ | A. 全量替换为统一 SVG 图标集<br>B. 仅替换「UI 控件类」图标（导航/按钮/状态），「文件类型类」emoji 保留<br>C. 保持现状 | 影响 R52；A 工作量最大但一致性最好 | 建议 **B**（文件类型 emoji 有信息价值，UI 控件必须统一） |
| **Q4** | 是否删除 4 个零引用组件（1,420 行：`ExplorerPage` / `FloatingInput` / `CodeBlockExecutor` / `Diagnostic`）？ | A. 删除<br>B. 保留但标记 `@deprecated` 并排除在改造范围外<br>C. 保留并一并改造 | 影响 R47、R53；A 可显著减少本次改造面积 | 建议 **A**（但需确认无外部/后续计划引用；若担心可先 `git` 保留历史） |
| **Q5** | 会话气泡非对称圆角（`MessageNode.vue:1063` 的 `14px 14px 4px 14px`）是否保留？ | A. 保留（作为会话区唯一例外）<br>B. 对齐 `--radius-*` 统一为对称圆角 | 影响 R19 | 建议 **A**（聊天气泡的方向性圆角是成熟设计语言，且属已完成改造的一部分，不应回退） |
| **Q6** | 代码编辑器（CodeEditor / MarkdownEditor）在浅色主题下是否应保持深色？ | A. 跟随主题（浅色主题用浅色编辑器）<br>B. 始终保持深色（类 VS Code 默认）<br>C. 跟随主题，但提供「编辑器独立配色」开关 | 影响 R43；B 下需向用户说明这是刻意为之 | 建议 **C**（默认跟随主题，保留深色选项） |
| **Q7** | 深色对比度的取舍：`--accent` 浅色从 `#667eea` → `#4f46e5` 会改变品牌观感，是否接受？ | A. 接受（换取 AA 达标）<br>B. 不接受，保持 `#667eea`，接受主按钮白字 3.66:1<br>C. 保持 `#667eea` 作填充，但引入 `--accent-text: #4f46e5` 专用于前景文字 | 影响 R04、C3、C4 | 建议 **A**（最彻底）；若品牌敏感则 **C**（但 C 下主按钮白字仍 3.66:1，需同时在深色主按钮上使用深色前景） |
| **Q8** | `--text-muted` 从 `#999999` → `#6b7280`（浅）、`#64748b` → `#7c8595`（深），会让次级文字更「重」，是否接受？ | A. 接受（换取 AA 达标）<br>B. 保持浅灰观感，接受 2.85:1 | 影响 R04、C1、C2 | 建议 **A** |
| **Q9** | 三栏布局响应式改造的范围？ | A. 仅断点自适应（R49）<br>B. 断点 + 手动折叠（R49）<br>C. 再加可拖拽分栏（R49 + R50） | 影响工作量 | 建议 **B**（C 的拖拽交互收益边际递减） |
| **Q10** | 是否引入「视觉走查页」（`/design-review`，仅 dev 环境）作为长期基线？ | A. 引入<br>B. 不引入 | 影响 R55 | 建议 **A**（防止后续再次漂移，成本很低） |

---

## 附录 A：审计取证命令清单（供架构师复核）

```bash
cd D:\Bing\symbio\tauri

# 文件与规模
find src -name "*.vue" | wc -l                      # 38
find src -name "*.vue" -exec wc -l {} \; | sort -rn # 11,395 行
find src -name "*.css"                              # 0 —— 无全局样式表

# 硬编码色值
grep -rhoP '#[0-9a-fA-F]{3,6}\b' src --include=*.vue --exclude=App.vue | wc -l   # 299
grep -rc 'rgba(0, *0, *0' src --include=*.vue | grep -v ':0$'                    # 81 处 / 28 文件

# 令牌引用 vs 定义
grep -rhoP 'var\(\s*--[a-zA-Z0-9-]+' src | sed 's/var(\s*//' | sort -u > /tmp/used.txt
grep -rhoP '^\s*--[a-zA-Z0-9-]+\s*:' src --include=*.vue | grep -oP '\-\-[a-zA-Z0-9-]+' | sort -u > /tmp/defined.txt
comm -23 /tmp/used.txt /tmp/defined.txt   # 未定义：--border-color --color-border-subtle
                                           #          --color-primary-hover --color-text-primary --font-mono
comm -13 /tmp/used.txt /tmp/defined.txt   # 未使用：--card-pad-y --color-msg-card-border --header-height

# 无障碍
grep -rn "focus-visible" src --include=*.vue | wc -l   # 0
grep -rn 'aria-label'   src --include=*.vue | wc -l    # 4
grep -rn 'role="'       src --include=*.vue | wc -l    # 4
grep -rn 'tabindex'     src --include=*.vue | wc -l    # 1

# 重复定义
grep -rn '^\.toast {'       src --include=*.vue   # 4 处
grep -rn '^\.icon-btn {'    src --include=*.vue   # 8 处
grep -rn '^\.status-dot {'  src --include=*.vue   # 5 处
grep -rn '@keyframes pulse' src --include=*.vue   # 8 处

# 死代码（零 import）
for f in ExplorerPage FloatingInput CodeBlockExecutor Diagnostic; do
  echo "--- $f ---"; grep -rn "$f" src --include=*.vue --include=*.ts | grep -v "components/$f.vue"
done   # 全部为空

# 原生弹窗
grep -rn '[^.a-zA-Z]\(confirm\|alert\|prompt\)(' src --include=*.vue   # 9 处（含 1 处死代码）
```

## 附录 B：关键 `文件:行` 速查

| 主题 | 位置 |
| --- | --- |
| 令牌定义（浅色） | `App.vue:18-74` |
| 令牌定义（深色） | `App.vue:116-171` |
| 仅深色存在的 4 个令牌 | `App.vue:167-170` |
| 对比度不达标令牌 | `App.vue:19`（`#667eea`）、`:26`（`#999999`）、`:124`（`#64748b`） |
| 齿轮图标 path 错误 | `MainLayout.vue:79` |
| 侧边栏硬编码 | `MainLayout.vue:209,210,244,249,254` |
| 白色下拉菜单 | `chat/ChatSettings.vue:531` |
| 近乎纯白的悬浮对话框 | `ModelSelectionDialog.vue:307` |
| 纯白对话框 | `SettingsPage.vue:458` |
| Unicode 转义未解码 | `SettingsPage.vue:115` |
| 死 CSS（约 20 个选择器） | `SettingsPage.vue:391-406,438-456` |
| 选中态用绿色 | `settings/ModelProviderCard.vue:113-114` |
| 状态点 failed 与 idle 同色 | `settings/McpServerCard.vue:150,160-161` |
| Toast 默认绿底 | `views/ModelProvidersView.vue:225` |
| 编辑器硬编码深色 | `CodeEditor.vue:174,175,183,194,195,208,212` |
| 树节点硬编码 | `FileTreeNode.vue:166,170` |
| 三栏固定不收缩 | `views/SessionView.vue:41-56` |
| 字号档位实现 | `stores/appearance.ts:27-31,61` |
| 路由定义 | `router/index.ts:9-26` |
