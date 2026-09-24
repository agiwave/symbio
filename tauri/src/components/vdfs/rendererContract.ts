/**
 * 详情渲染器统一契约（VDFS）
 *
 * 三栏工作台按**同一组 props / 事件**装配所有详情渲染器，因此「渲染器要接收
 * 什么」必须只有一份。此前它只以散文形式写在每个渲染器的头注释里
 * （「渲染器统一契约（详见 VdfsTextDetail 同名说明）」）——「哪些 prop 存在」
 * 靠人去逐个比对，注释与代码不一致时没有任何东西会发现。
 *
 * ## props 分三类，各有归属
 *
 * | 类别 | 字段 | 谁生产 |
 * |---|---|---|
 * | 条目 | `node` | 页面（useVdfs 的选中项 = `VdfsItem`） |
 * | 渲染数据 | `data` / `error` / `fieldErrors` / `saving` / `testing` | 页面 |
 * | 机制动作 | `mechanismActions` / `mechanismBusy` | 页面**单点算好**（useVdfs.mechanismActions） |
 *
 * ## 为什么「未用到的项也要声明」
 *
 * Vue 会把**未声明**的 attribute 透传到组件根元素上。页面恒传 `node` / `data` /
 * `testing` / `mechanism-actions`…，渲染器不声明它们，它们就会作为 DOM 属性落到
 * 根节点（`<div testing="false">`）。此前 Appearance / About 靠
 * `inheritAttrs: false` 单独兜住，而 VdfsTextDetail / VdfsReadonlyDetail 一直在
 * 往 DOM 上写机制字段。声明全量 = 契约完整 = 不需要任何兜底。
 */

import type { DetailAction, VdfsFieldError, VdfsItem } from '@/schemas/vdfs'

export interface VdfsRendererProps {
  /**
   * 被呈现的**条目**（机制保证：渲染器只在有选中项时挂载，故恒非空）。
   *
   * 是 `VdfsItem`（地址 + 节点）而不是 `VdfsNode`：渲染器要按地址回读 / 写回，
   * 而纯节点不带地址。草稿（新建态）是唯一没有地址的一项——`path` 为空串，
   * 判据见 `schemas/vdfs.isVdfsDraft`。
   */
  node: VdfsItem
  /**
   * 渲染器数据：form → 字段值对象；文本类（含 message 的正文）→ 文本；
   * 其余不传。形状由节点的 `ext` → 渲染器映射决定（`registry/vdfsTypes`）。
   */
  data?: unknown
  /** 详情级错误（纯文本） */
  error?: string
  /** 字段级校验错误（provider 自持校验的产物；机制不解释其含义） */
  fieldErrors?: VdfsFieldError[]
  /** 写操作在途（保存 / 删除 / 重命名共用；还兼作列表的锁） */
  saving?: boolean
  /** 节点动作执行中（如「测试连接」；与 saving 分开：动作不写数据） */
  testing?: boolean
  /** 机制级默认动作（重命名 / 删除），由页面单点计算后注入 */
  mechanismActions?: DetailAction[]
  /** 正在执行的机制动作 id（驱动其 `busy_label`；同时最多一个） */
  mechanismBusy?: string | null
}
