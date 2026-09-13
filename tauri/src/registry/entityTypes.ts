/**
 * 实体类型注册表（前端纯展示层）—— editor 组件 + icon 按 kind（或 kind:ext）注册
 *
 * 分层原则：
 * - 资源的**存在性/能力/寻址/顺序/标签**一律来自 VDFS（`.vdfs` 挂载点与节点），
 *   前端不再硬编码类型清单（S5 起实体页与 `entities/providers` 均已下线）；
 * - 本模块只维护**前端 UI 专属**的映射：某 kind 的专属编辑表单（editor 组件）与
 *   SVG 图标。后端不参与下发 Vue 组件 / SVG。
 *
 * ## 注册键：kind 级 与 项级（kind:ext）
 *
 * - `registerEntityEditor(kind, 组件)`：类型级（该 kind 所有实体共用，如 model）；
 * - `registerEntityEditor('kind:ext', 组件)`：项级"扩展名"分发——同一 kind 下
 *   不同实体项按 `item.config_type`（后端 extra 字段）进入不同 editor，
 *   类似文件系统"不同扩展名打开不同编辑器"（如 setting 的各设置分区）。
 *
 * ## 新增一种资源类型的两步扩展位
 *
 * 1. 后端：实现 `EntityProvider` + 在 `provider_registry()` 登记一条（或直接实现
 *    `VdfsProvider` 并挂载）——`.vdfs/<kind>` 自动多一个挂载点，导航与能力
 *    随之生成（`entities/*` 协议已于 S11 下线）；
 * 2. 前端（可选）：`registerEntityEditor(...)` 与
 *    `registerEntityIcon(...)`——未注册的走 VDFS 机制级兜底渲染器
 *    （`registry/vdfsRenderers`：表单 / 文本 / 只读）。
 */

import { defineComponent, h, markRaw, shallowReactive, type Component } from 'vue'
import Session from '@/components/entities/Session.vue'

/** 编辑器/图标查找目标：kind + 可选"扩展名"（后端 extra.config_type，unknown 兼容索引签名） */
export interface EntityRegistryTarget {
  kind: string
  config_type?: unknown
}

/** 提取项级扩展名（仅接受 string，其余忽略） */
function extOf(target: EntityRegistryTarget): string | null {
  return typeof target.config_type === 'string' && target.config_type ? target.config_type : null
}

/** 前端展示登记：某 kind（或 kind:ext）的专属编辑器，未登记走通用兜底 */
const editors = shallowReactive<Record<string, Component>>({})

export function registerEntityEditor(kind: string, component: Component): void {
  editors[kind] = component
}

/** 类型级查找（新建模式等仅知 kind 的场景） */
export function getEntityEditor(kind: string): Component | undefined {
  return editors[kind]
}

/**
 * 项级查找：优先 `kind:ext`（扩展名决定编辑器），回退 kind。
 * ext 取 `item.config_type`（后端 extra 下发）。
 */
export function getEntityEditorFor(target: EntityRegistryTarget): Component | undefined {
  const ext = extOf(target)
  if (ext) {
    const keyed = editors[`${target.kind}:${ext}`]
    if (keyed) return keyed
  }
  return editors[target.kind]
}

/** kind（或 kind:ext）→ 自定义图标组件（未注册走默认图标） */
const icons = shallowReactive<Record<string, Component>>({})

export function registerEntityIcon(kind: string, icon: Component): void {
  icons[kind] = icon
}

export function getEntityIcon(kind: string): Component | undefined {
  return icons[kind]
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
  // 进入容器实体管理（外部链接）
  'open-container':
    '<path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>',
}

/** 动作图标 SVG path（icon 名优先于动作 id；无映射返回 undefined → 文字按钮） */
export function getActionIcon(action: { icon?: string; id: string }): string | undefined {
  return (action.icon && ACTION_ICONS[action.icon]) || ACTION_ICONS[action.id]
}

/** 项级图标查找：优先 `kind:ext`，回退 kind */
export function getEntityIconFor(target: EntityRegistryTarget): Component | undefined {
  const ext = extOf(target)
  if (ext) {
    const keyed = icons[`${target.kind}:${ext}`]
    if (keyed) return keyed
  }
  return icons[target.kind]
}

// ============ 内置注册 ============

// model / mcp / skill 详情与新建、agent bundle 概览、设置三分区均不注册：
// 由后端 `entities/detail` 下发定义，DetailForm 通用渲染器动态生成
// （definition-driven detail）。

// Session（会话）：kind 级注册——详情 = 聊天工作区（ChatMainPanel）；
// 本注册存在 → 该 kind 用专属 editor 渲染（VDFS 的 ext 分发之外的前端补充）
// 进入该 editor 的引导态（新建会话），列表/删除由机制承担（delete_item 钩子）。
registerEntityEditor('session', markRaw(Session))

// 设置分区（setting）已迁移到 VDFS（/vdfs/setting）：
// session/local/web/gateway 走 ext=form 的定义驱动表单；appearance/about 的
// 专属面板改由 VDFS 渲染器注册表装配（registry/vdfsRenderers.ts）。
// 此处不再注册 setting 的 editor —— 实体机制侧已退场。

/** 用 SVG path 构造轻量图标组件（feather 风格线条图标） */
function svgIcon(inner: string): Component {
  return markRaw(
    defineComponent({
      name: 'EntitySvgIcon',
      render() {
        return h('svg', {
          viewBox: '0 0 24 24',
          width: 16,
          height: 16,
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
registerEntityIcon(
  'setting:appearance',
  svgIcon(
    '<line x1="4" y1="21" x2="4" y2="14"/><line x1="4" y1="10" x2="4" y2="3"/><line x1="12" y1="21" x2="12" y2="12"/><line x1="12" y1="8" x2="12" y2="3"/><line x1="20" y1="21" x2="20" y2="16"/><line x1="20" y1="12" x2="20" y2="3"/><line x1="1" y1="14" x2="7" y2="14"/><line x1="9" y1="8" x2="15" y2="8"/><line x1="17" y1="16" x2="23" y2="16"/>'
  )
)
registerEntityIcon(
  'setting:session',
  svgIcon(
    '<path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/>'
  )
)
registerEntityIcon(
  'setting:local',
  svgIcon('<polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/>')
)
registerEntityIcon(
  'setting:web',
  svgIcon(
    '<circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>'
  )
)
registerEntityIcon(
  'setting:gateway',
  svgIcon(
    '<path d="M4 12h4l2-5 4 10 2-5h4"/><circle cx="12" cy="12" r="10"/>'
  )
)
registerEntityIcon(
  'setting:about',
  svgIcon(
    '<circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/>'
  )
)

// 设置入口的展示图标（左侧导航的挂载点来自 `vdfs/providers`）
registerEntityIcon(
  'setting',
  svgIcon(
    '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>'
  )
)

// 场景子类别图标（provider 场景自定的 kind 级图标，与主界面图标体系同构）
registerEntityIcon(
  'prompt',
  svgIcon(
    '<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><polyline points="13 2 13 9 20 9"/><line x1="9" y1="13" x2="15" y2="13"/><line x1="9" y1="17" x2="15" y2="17"/>'
  )
)
// 会话容器「目录树」子类别的树节点图标（provider 场景注册；tree 机制按
// 节点 kind + config_type（directory/file）项级分发，机制本身不含文件语义）
registerEntityIcon(
  'dir:directory',
  svgIcon(
    '<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>'
  )
)
registerEntityIcon(
  'dir:file',
  svgIcon(
    '<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><polyline points="13 2 13 9 20 9"/>'
  )
)

// ============ 主导航 kind 级图标（6 类并排时各自独立，不再共用默认文件图标） ============
registerEntityIcon(
  'session',
  svgIcon(
    '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>'
  )
)
registerEntityIcon(
  'model',
  svgIcon(
    '<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="3"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>'
  )
)
registerEntityIcon(
  'mcp',
  svgIcon(
    '<rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><line x1="6" y1="6" x2="6.01" y2="6"/><line x1="6" y1="18" x2="6.01" y2="18"/>'
  )
)
registerEntityIcon(
  'agent',
  svgIcon(
    '<path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>'
  )
)
registerEntityIcon(
  'skill',
  svgIcon(
    '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>'
  )
)
