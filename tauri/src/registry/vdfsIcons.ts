/**
 * VDFS 前端图标注册表（纯 UI 映射）—— icon 按 kind（或 kind:ext）登记
 *
 * 分层原则（与 registry/vdfsTypes.ts / vdfsRenderers.ts 同一套）：
 * - 资源的**存在性/能力/寻址/顺序/标签**一律来自 VDFS（`<根>` 挂载点与节点），
 *   前端不硬编码类型清单；
 * - 本模块只维护**前端 UI 专属**的映射：某资源类别的 SVG 图标、动作按钮图标，
 *   以及详情**字段**的 emoji 图标（后端不参与下发 SVG path / emoji）。
 *
 * 详情渲染器的登记不在这里 —— 那是 `registry/vdfsRenderers.ts`（标识 → 组件）。
 *
 * ## 登记键：kind 级 与 项级（kind:ext）
 *
 * - `registerVdfsIcon(kind, 组件)`：类型级（该 kind 下所有节点共用，如 model）；
 * - `registerVdfsIcon('kind:ext', 组件)`：项级"扩展名"分发——同一 kind 下不同节点
 *   按节点的项级标识（如设置分区的 `config_type`）进入不同图标，类似文件系统
 *   「不同扩展名用不同图标」。
 *
 * ## 新增一类资源的两步扩展位
 *
 * 1. 后端：实现 `VdfsProvider` 并挂载 —— `<根>/<插件名>` 自动多一个挂载点，
 *    导航与能力随之生成；
 * 2. 前端（可选）：`registerVdfsIcon(...)`——未登记的走默认图标（侧栏 /
 *    VdfsCard 各自兜底），详情渲染器未登记时走机制级只读兜底。
 */

import { defineComponent, h, markRaw, shallowReactive, type Component } from 'vue'

/** 图标查找目标：kind + 可选"扩展名"（节点的场景扩展字段，unknown 兼容索引签名） */
export interface VdfsIconTarget {
  kind: string
  config_type?: unknown
}

/** 提取项级扩展名（仅接受 string，其余忽略） */
function extOf(target: VdfsIconTarget): string | null {
  return typeof target.config_type === 'string' && target.config_type ? target.config_type : null
}

/** kind（或 kind:ext）→ 自定义图标组件（未登记走默认图标） */
const icons = shallowReactive<Record<string, Component>>({})

export function registerVdfsIcon(kind: string, icon: Component): void {
  icons[kind] = icon
}

/**
 * **取图标这件事的唯一实现**（侧栏 / 列表卡片 / 任何要按节点出图的地方都调它）。
 *
 * ## 回退链：**具体在前、笼统在后**
 *
 * 1. `kind:config_type` —— 项级（同一 kind 下按分区/扩展名分图）
 * 2. `config_type`      —— 配置键（部分分区把图标登记在**裸分区名**上：
 *                          `appearance` / `session` / `about` …）
 * 3. `name`             —— 名单级（`<根>` 挂载点：`session` / `model` / …）
 * 4. `kind`             —— 类型级（**笼统的容器形态**：`dir` / `file`，最后兜底）
 *
 * ## 顺序就是规则本身（**改动前必读**）
 *
 * `kind` 的取值分两类：**具体的身份**（`session` / `model` …）与**笼统的容器
 * 形态**（`dir` / `file`）。「优先」只对前者成立——后者对区分同类成员**零信息量**。
 *
 * `<根>` 的六个挂载点由 `VdfsNode::dir(...)` 产出，`kind` **全是 `"dir"`**
 * （后端自测 `composite/vdfs.test.rs` 断言了此值）。曾把顺序写成 kind 优先，
 * 六个挂载点**全部**命中 `dir` 那张图 → 主界面左栏整排退成同一个默认文件夹。
 * 完整事故记录见 `docs/design/frontend-ui-ux-plan.md` §9。
 *
 * 从前还有两个中间层 `getVdfsIcon` / `getVdfsIconFor`，各自带一条「回退到
 * `icons[kind]`」的尾巴——那条尾巴会抢先命中笼统 kind，把名字查询挡死。已删除，
 * **别再引入**：多一个入口就多一处能写错顺序的地方。
 *
 * 护栏见 `registry/__tests__/vdfsIcons.spec.ts`（夹具必须用真实 `kind: 'dir'`）
 * 与 `composables/__tests__/useVdfs.spec.ts`（六个挂载点图标须互不相同）。
 *
 * @param node  任何带 `kind` / `name` / 可选 `config_type` 的节点
 * @returns     命中的图标组件；全未命中返回 `undefined`（由调用方决定兜底）
 */
export function iconForNode(node: {
  kind?: string
  name?: string
  config_type?: unknown
}): Component | undefined {
  const kind = node.kind ?? ''
  const name = node.name ?? ''
  const ext = extOf(node as VdfsIconTarget)
  return (
    (ext ? icons[`${kind}:${ext}`] : undefined) ?? // 1. 项级
    (ext ? icons[ext] : undefined) ?? //             2. 配置键（名字空间）
    (name ? icons[name] : undefined) ?? //           3. 名单级（挂载点）
    (kind ? icons[kind] : undefined) //              4. 类型级（笼统容器，最后兜）
  )
}

// ============ 动作图标（DetailAction icon/id → SVG path，纯 UI 映射） ============
//
// 机制动作的图标优先渲染：语义动作 id 自带默认图标；定义可用 icon 字段
// 指定其他图标名（如区分同为 save 的「跳过校验保存」）；无映射 → 文字按钮。

const ACTION_ICONS: Record<string, string> = {
  // 保存（软盘）
  save:
    '<path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"/><polyline points="17 21 17 13 7 13 7 21"/><polyline points="7 3 7 8 15 8"/>',
  // 跳过校验保存（盾牌斜杠）
  'save-skip':
    '<path d="M19.69 14a6.9 6.9 0 0 0 .31-2V5l-8-3-8 3v6c0 5.55 3.84 10.74 9 12 2.43-.61 4.5-2.02 5.91-4"/><line x1="1" y1="1" x2="23" y2="23"/>',
  // 连接测试（闪电）
  test: '<polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/>',
  // 删除（垃圾桶）
  delete:
    '<polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>',
  // 设为默认（星标）
  'set-default':
    '<polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>',
  // 浏览节点内部 —— 进入**下一级**（同名目录下的子结构）
  //
  // 语义是「往下钻一层」，不是「跳到站外」：原先是外部链接形（方框 + 右上箭头），
  // 与「分享 / 在新窗口打开」同形，用户会读成把资源发出去。改为「进入」形
  // （圆 + 右向 chevron），与左上角返回键（左向 chevron）恰好是一对逆操作，
  // 且与列表项「点目录即钻入」的心智一致。
  'open-container':
    '<circle cx="12" cy="12" r="9"/><polyline points="10.5 8.5 14 12 10.5 15.5"/>',
}

/** 动作图标 SVG path（icon 名优先于动作 id；无映射返回 undefined → 文字按钮） */
export function getActionIcon(action: { icon?: string; id: string }): string | undefined {
  return (action.icon && ACTION_ICONS[action.icon]) || ACTION_ICONS[action.id]
}

// ============ 详情字段图标（emoji 映射，纯 UI 资产） ============
//
// 与上面的 SVG 图标同一分层定位：**后端下发图标名，前端负责把名字映射为具体
// 视觉**。后端不参与下发 emoji/SVG。
//
// 为什么是 emoji 而不是 SVG：字段图标出现在**紧凑选项栏**（会话输入区下方那一排
// 小按钮）里，那里从第一版起就是 emoji 语言，换成线条图标是视觉回归。
//
// 这份表原先住在 `registry/optionIcons.ts`（选项机制的专属文件）。机制下线后
// 「图标名 → 视觉」仍要有唯一一处，故并入本文件——`registry/optionIcons.ts` 删除，
// 取值一字未改。
//
// 未登记的图标名回落默认图标；字段仍可正常渲染（只是图标为通用形），
// 因此新增贡献方无需改动本文件。

/** 字段图标名 → 视觉（emoji） */
const FIELD_ICONS: Record<string, string> = {
  folder: '📁',
  agent: '🎭',
  model: '🧠',
  risk: '🎯',
  'run-mode': '💬',
  heartbeat: '⏰',
  play: '▶',
}

/** 未登记字段图标名的回落 */
export const DEFAULT_FIELD_ICON = '⚙'

/** 解析字段图标名（未知回落默认图标） */
export function fieldIcon(name?: string): string {
  if (!name) return DEFAULT_FIELD_ICON
  return FIELD_ICONS[name] ?? DEFAULT_FIELD_ICON
}

// ============ 内置登记 ============

// model / mcp / skill / agent 与设置各分区的详情都不在前端登记组件：
// 由后端 `detail_definition` 下发定义（随 VDFS 节点 `schema`），
// DetailForm 通用渲染器动态生成（definition-driven detail）。

/** 用 SVG path 构造轻量图标组件（feather 风格线条图标） */
function svgIcon(inner: string): Component {
  return markRaw(
    defineComponent({
      name: 'VdfsSvgIcon',
      render() {
        return h('svg', {
          viewBox: '0 0 24 24',
          // **不写死 width/height**：同一张图标会同时出现在侧栏（20px）与列表
          // 卡片（16px）两个容器里。写死任何一边，另一边就是错的——表现为
          // 同一排图标大小不一。尺寸一律交给容器 CSS：`.nav-btn svg` / `.card-icon`。
          fill: 'none',
          stroke: 'currentColor',
          'stroke-width': 2,
          'stroke-linecap': 'round',
          'stroke-linejoin': 'round',
          innerHTML: inner,
        })
      },
    })
  )
}

// 设置分区图标
registerVdfsIcon(
  'plugin_manager:appearance',
  svgIcon(
    '<line x1="4" y1="21" x2="4" y2="14"/><line x1="4" y1="10" x2="4" y2="3"/><line x1="12" y1="21" x2="12" y2="12"/><line x1="12" y1="8" x2="12" y2="3"/><line x1="20" y1="21" x2="20" y2="16"/><line x1="20" y1="12" x2="20" y2="3"/><line x1="1" y1="14" x2="7" y2="14"/><line x1="9" y1="8" x2="15" y2="8"/><line x1="17" y1="16" x2="23" y2="16"/>'
  )
)
registerVdfsIcon(
  'plugin_manager:session',
  svgIcon(
    '<path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/>'
  )
)
registerVdfsIcon(
  'plugin_manager:local',
  svgIcon('<polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/>')
)
registerVdfsIcon(
  'plugin_manager:web',
  svgIcon(
    '<circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>'
  )
)
registerVdfsIcon(
  'plugin_manager:gateway',
  svgIcon(
    '<path d="M4 12h4l2-5 4 10 2-5h4"/><circle cx="12" cy="12" r="10"/>'
  )
)
registerVdfsIcon(
  'plugin_manager:telegram',
  svgIcon('<line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/>')
)
registerVdfsIcon(
  'plugin_manager:about',
  svgIcon(
    '<circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/>'
  )
)

// 插件管理入口的展示图标（左侧导航的挂载点来自 `vdfs/list` 的 `<根>` 子目录，
// 挂载名 = 插件名 `plugin_manager`）
registerVdfsIcon(
  'plugin_manager',
  svgIcon(
    '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>'
  )
)

// 场景子类别图标（provider 场景自定的 kind 级图标，与主界面图标体系同构）
registerVdfsIcon(
  'prompt',
  svgIcon(
    '<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><polyline points="13 2 13 9 20 9"/><line x1="9" y1="13" x2="15" y2="13"/><line x1="9" y1="17" x2="15" y2="17"/>'
  )
)
// 「目录树」子类别的树节点图标（provider 场景登记；树按节点 kind +
// config_type（directory/file）做项级分发，机制本身不含文件语义）
//
// **kind 级 `dir` 也要登记**：树节点恒带 `config_type`（走项级键），但树**根**
// （`<sid>/workdir`）不带——它落到 kind 级。不登记就会退成文件夹兜底，
// 于是「工作目录」在侧栏与其他区段混作一谈。
registerVdfsIcon(
  'dir',
  svgIcon(
    '<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/><line x1="12" y1="11" x2="12" y2="17"/><line x1="9" y1="14" x2="15" y2="14"/>'
  )
)
registerVdfsIcon(
  'dir:directory',
  svgIcon(
    '<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>'
  )
)
registerVdfsIcon(
  'dir:file',
  svgIcon(
    '<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><polyline points="13 2 13 9 20 9"/>'
  )
)

// ============ 主导航 kind 级图标（6 类并排时各自独立，不再共用默认文件图标） ============
registerVdfsIcon(
  'session',
  svgIcon(
    '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>'
  )
)
registerVdfsIcon(
  'model',
  svgIcon(
    '<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="3"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>'
  )
)
registerVdfsIcon(
  'mcp',
  svgIcon(
    '<rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><line x1="6" y1="6" x2="6.01" y2="6"/><line x1="6" y1="18" x2="6.01" y2="18"/>'
  )
)
registerVdfsIcon(
  'agent',
  svgIcon(
    '<path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>'
  )
)
registerVdfsIcon(
  'skill',
  svgIcon(
    '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>'
  )
)
// 记忆挂载点（`<根>/memory`：智能体自身 + 工作区两份记忆）：摊开的书——长期翻开的那页
registerVdfsIcon(
  'memory',
  svgIcon(
    '<path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/>'
  )
)

// ============ 会话内部区段的 kind 级图标 ============
//
// 进到 `<根>/session/<id>` 之后，左栏列出的是会话自己的四五个区段。它们**没有**
// 顶层 kind 同名的段名可用（段名是 `message` / `inbox` / `MEMORY.md` /
// `subsession` / `workdir`），因此按 kind 登记——后端 `words.rs` 把 `messages` /
// `inbox` 声明为协议词，`workdir/mod.rs` 为另两个区段补了同样的词。
//
// 这四张图各自要说清「这一栏是干什么的」，不能共用文件夹兜底：
// 用户在会话里的第一眼就是这排图标，四个一样的图标等于没有导航。

/** 转写列表（`kind = messages`）：对话气泡——已经发生的一问一答 */
registerVdfsIcon(
  'messages',
  svgIcon(
    '<path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/>'
  )
)
/** 收件箱（`kind = inbox`）：信封——还没被消费的那句 */
registerVdfsIcon(
  'inbox',
  svgIcon(
    '<path d="M4 4h16a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z"/><polyline points="22 7 12 13 2 7"/>'
  )
)
/** 子会话（`kind = subsession`）：分叉的对话——本会话派出去的分支 */
registerVdfsIcon(
  'subsession',
  svgIcon(
    '<circle cx="6" cy="6" r="3"/><circle cx="18" cy="18" r="3"/><path d="M6 9v3a3 3 0 0 0 3 3h6"/><polyline points="15 12 18 15 15 18"/>'
  )
)
