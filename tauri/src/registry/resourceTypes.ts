/**
 * 资源类型注册表（前端纯展示层）—— editor 组件 + icon 按 kind（或 kind:ext）注册
 *
 * 分层原则（见 composables/useResourceProviders.ts）：
 * - 类型的**存在性/能力/前缀/顺序/标签**全部来自后端 `resources/providers` 下发的
 *   ProviderInfo（是权威，前端不再硬编码类型清单）；
 * - 本模块只维护**前端 UI 专属**的映射：某 kind 的专属编辑表单（editor 组件）与
 *   SVG 图标。后端不参与下发 Vue 组件 / SVG。
 *
 * ## 注册键：kind 级 与 项级（kind:ext）
 *
 * - `registerResourceEditor(kind, 组件)`：类型级（该 kind 所有资源共用，如 model）；
 * - `registerResourceEditor('kind:ext', 组件)`：项级"扩展名"分发——同一 kind 下
 *   不同资源项按 `item.config_type`（后端 extra 字段）进入不同 editor，
 *   类似文件系统"不同扩展名打开不同编辑器"（如 setting 的各设置分区）。
 *
 * ## 新增一种资源类型的两步扩展位
 *
 * 1. 后端：实现 ResourceProvider + 插件 route 接 dispatch + `provider_registry()`
 *    登记一条——前端即可自动发现（生成导航、进入统一资源页）；
 * 2. 前端（可选）：`registerResourceEditor(...)` 与
 *    `registerResourceIcon(...)`——未注册的走通用兜底
 *    （zip 面板 / JSON 编辑器 / 只读详情）。
 */

import { defineComponent, h, markRaw, shallowReactive, type Component } from 'vue'
import ModelProviderForm from '@/components/resources/ModelProviderForm.vue'
import AppearanceSettingsForm from '@/components/settings/AppearanceSettingsForm.vue'
import SessionConfigForm from '@/components/settings/SessionConfigForm.vue'
import LocalConfigForm from '@/components/settings/LocalConfigForm.vue'
import WebConfigForm from '@/components/settings/WebConfigForm.vue'
import AboutPanel from '@/components/settings/AboutPanel.vue'

/** 编辑器/图标查找目标：kind + 可选"扩展名"（后端 extra.config_type，unknown 兼容索引签名） */
export interface ResourceRegistryTarget {
  kind: string
  config_type?: unknown
}

/** 提取项级扩展名（仅接受 string，其余忽略） */
function extOf(target: ResourceRegistryTarget): string | null {
  return typeof target.config_type === 'string' && target.config_type ? target.config_type : null
}

/** 前端展示登记：某 kind（或 kind:ext）的专属编辑器，未登记走通用兜底 */
const editors = shallowReactive<Record<string, Component>>({})

export function registerResourceEditor(kind: string, component: Component): void {
  editors[kind] = component
}

/** 类型级查找（新建模式等仅知 kind 的场景） */
export function getResourceEditor(kind: string): Component | undefined {
  return editors[kind]
}

/**
 * 项级查找：优先 `kind:ext`（扩展名决定编辑器），回退 kind。
 * ext 取 `item.config_type`（后端 extra 下发）。
 */
export function getResourceEditorFor(target: ResourceRegistryTarget): Component | undefined {
  const ext = extOf(target)
  if (ext) {
    const keyed = editors[`${target.kind}:${ext}`]
    if (keyed) return keyed
  }
  return editors[target.kind]
}

/** kind（或 kind:ext）→ 自定义图标组件（未注册走默认图标） */
const icons = shallowReactive<Record<string, Component>>({})

export function registerResourceIcon(kind: string, icon: Component): void {
  icons[kind] = icon
}

export function getResourceIcon(kind: string): Component | undefined {
  return icons[kind]
}

/** 项级图标查找：优先 `kind:ext`，回退 kind */
export function getResourceIconFor(target: ResourceRegistryTarget): Component | undefined {
  const ext = extOf(target)
  if (ext) {
    const keyed = icons[`${target.kind}:${ext}`]
    if (keyed) return keyed
  }
  return icons[target.kind]
}

// ============ 内置注册 ============

// model 使用独立表单
registerResourceEditor('model', markRaw(ModelProviderForm))

// 设置分区：同一 kind（setting）下按 config_type 进入不同 editor
registerResourceEditor('setting:appearance', markRaw(AppearanceSettingsForm))
registerResourceEditor('setting:session', markRaw(SessionConfigForm))
registerResourceEditor('setting:local', markRaw(LocalConfigForm))
registerResourceEditor('setting:web', markRaw(WebConfigForm))
registerResourceEditor('setting:about', markRaw(AboutPanel))

/** 用 SVG path 构造轻量图标组件（feather 风格线条图标） */
function svgIcon(inner: string): Component {
  return markRaw(
    defineComponent({
      name: 'ResourceSvgIcon',
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
registerResourceIcon(
  'setting:appearance',
  svgIcon(
    '<line x1="4" y1="21" x2="4" y2="14"/><line x1="4" y1="10" x2="4" y2="3"/><line x1="12" y1="21" x2="12" y2="12"/><line x1="12" y1="8" x2="12" y2="3"/><line x1="20" y1="21" x2="20" y2="16"/><line x1="20" y1="12" x2="20" y2="3"/><line x1="1" y1="14" x2="7" y2="14"/><line x1="9" y1="8" x2="15" y2="8"/><line x1="17" y1="16" x2="23" y2="16"/>'
  )
)
registerResourceIcon(
  'setting:session',
  svgIcon(
    '<path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/>'
  )
)
registerResourceIcon(
  'setting:local',
  svgIcon('<polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/>')
)
registerResourceIcon(
  'setting:web',
  svgIcon(
    '<circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>'
  )
)
registerResourceIcon(
  'setting:about',
  svgIcon(
    '<circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/>'
  )
)

/** 构造资源路径唯一标识：`[provider]/[id].[kind]`（如 `model/openai.model`） */
export function resourcePath(provider: string, id: string, kind: string): string {
  return `${provider}/${id}.${kind}`
}
