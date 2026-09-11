/**
 * 级联选项机制协议（前端侧类型契约）
 *
 * 与后端对齐：symbio/src/symbio_core/schemas/options.rs
 *
 * ## 机制
 *
 * 会话页输入区下方的选项行由后端下发（选项宿主 = session 插件，
 * `worker/session/options/list`）。前端只实现一套通用渲染机制
 * （`ChatOptionBar` + `useSessionOptions`），不含任何具体业务选项。
 *
 * 三类选项：
 * - `invoke`：调用特定后端服务（`action.endpoint`）；
 * - `sub`：子选项列表（级联；`children` 内联或经 `parent` 懒加载）；
 * - `form`：自动化表单（复用 `DetailDefinition` / `DetailForm`）。
 */

import type { DetailDefinition } from './entities'

/** 选项节点类型 */
export type OptionType = 'invoke' | 'sub' | 'form'

/**
 * 选项栏（会话输入区下方）显示策略 —— 机制级、由后端声明，前端零写死。
 *
 * - `show_label`：是否在选项栏显示类别标签（label）。缺省 true（现行行为）；
 *   false = 仅「图标 + 当前值」，类别名整体移入悬停提示。后端据此统一控制紧凑度。
 */
export interface OptionDisplay {
  show_label?: boolean
}

/**
 * 选项动作 —— invoke / form 的执行规格。
 *
 * - `endpoint`：后端服务路径（`callPlugin` 调用）；
 * - `payload`：固定参数（动态参数合并其上）；
 * - `pick` + `bind`：机制原生取值原语（闭集 `directory` | `file`）与其写入的
 *   参数点路径。`form` 保存时表单字段值同样写入 `bind`。
 */
export interface OptionAction {
  endpoint: string
  payload?: Record<string, unknown>
  /** 机制原生取值原语（后端无法唤起原生对话框） */
  pick?: 'directory' | 'file'
  /** 动态参数的写入路径（点路径，如 `metadata.workdir`） */
  bind?: string
}

/**
 * 选项节点。
 *
 * 显示信息：`label` / `icon` / `description` / `value` / `value_label`；
 * 状态信息：`status` / `status_detail` / `enabled`；
 * 类型专属：`action`（invoke/form）/ `children`（sub）/ `form` + `data`（form）。
 */
export interface OptionNode {
  id: string
  /** 显示名（如「工作目录」「智能体」） */
  label: string
  /** 图标名（前端 UI 资产映射；缺省回落默认图标） */
  icon?: string
  description?: string
  option_type: OptionType
  /** 展示顺序（升序） */
  order: number
  /** 状态：active | working | disabled | error | unknown */
  status: string
  status_detail?: string
  /** 当前选中值（状态型选项；sub 子项自身取值与父节点 value 对应） */
  value?: string
  /** 当前选中值的展示文本（缺省 = value） */
  value_label?: string
  /** 是否可选（false = 只读展示） */
  enabled?: boolean
  action?: OptionAction
  /** form：表单定义（与实体详情表单同一套 schema） */
  form?: DetailDefinition
  /** form：表单初始数据（字段名 → 值） */
  data?: Record<string, unknown>
  /** sub：子选项（内联；空/缺省 = 经 parent 懒加载） */
  children?: OptionNode[]
  /** 选项栏显示策略（机制级；由后端声明，前端据此渲染，不写死） */
  display?: OptionDisplay
}

/** `options/list` 请求（`parent` 缺省 = 根层） */
export interface OptionsRequest {
  session_id?: string
  parent?: string
}

/** `options/list` 响应 */
export interface OptionsResponse {
  nodes: OptionNode[]
}

/** 状态取值（与实体机制 §2.4 同一语义） */
export const OPTION_STATUS = {
  active: 'active',
  working: 'working',
  disabled: 'disabled',
  error: 'error',
  unknown: 'unknown',
} as const
