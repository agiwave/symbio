/**
 * reading — 读数面板的宿主方言（definition-driven reading）
 *
 * 与 `vdfs-form`（detail 方言）同一族：**后端下发描述，前端零业务知识地渲染**。
 * 差别只在用途——detail 服务可编辑详情，本方言服务**只读**的指标读数。
 *
 * 前端因此不出现任何会话/统计语义：这里只认分区、行、标签、值、口径提示与语气。
 * 换一个产者（成本、记忆、任何只读路由）前端零改动。
 *
 * 后端真源：`symbio/src/symbio_core/present/reading.rs`。
 * 跨栈镜像由 `scripts/protocol-mirror-audit.mjs` 的两张登记表看守：结构体在
 * `STRUCT_SETS`（`ReadingRow` / `ReadingSection` / `ReadingPanel`），闭集词表在
 * `ENUM_SETS`（`ReadingTone` ↔ `READING_TONES`）。字段或取值改名 ⇒ 该审计红。
 */

/**
 * 语气词表（跨栈镜像的**词表侧**）：后端 `ReadingTone` 的线上取值。
 *
 * 先有数组、再派生类型——守卫按数组逐词比对后端枚举；把类型写成裸联合而没有
 * 数组，后端加一档时前端不会红（本批补守卫的原因）。
 */
export const READING_TONES = ['normal', 'warn'] as const

/** 一列读数的语气（产者判，前端只换样式）。 */
export type ReadingTone = (typeof READING_TONES)[number]

/** 一行读数。 */
export interface ReadingRow {
  /** 标签（「兜底率」/「未收束轮」…） */
  label: string
  /** 已由产者排版的值的文本（含单位与精度——前端不做任何数值格式化） */
  value: string
  /** 口径提示（这个数怎么算出来的） */
  hint?: string
  /** 语气（缺省 = normal） */
  tone?: ReadingTone
}

/**
 * 一条**几何呈现**（分区的可选一格）：填充位置 + 若干参考线位置。
 *
 * 全是产者**已经除好的 0..1 比率**——渲染器只做 `ratio → %` 的单位换算，不除第二次
 * （除法有两处，改系数就只有一半界面跟着变）。每根线**是什么**由同分区的某一行说明，
 * 对应关系住在产者：这里不出现「水位」「阈值」「预算」任何一个业务词。
 */
export interface ReadingBar {
  /** 填充到的位置（缺省 = 没有分母，条不表态，而不是补一个 0） */
  level?: number
  /** 参考线位置（零到多条，按产者给的顺序画） */
  marks?: number[]
  /** 语气（缺省 = normal）：着色由产者判，渲染器不解析值的语义 */
  tone?: ReadingTone
}

/** 一个分区（标题可选）。 */
export interface ReadingSection {
  title?: string
  rows: ReadingRow[]
  /** 条形几何（可选）——不认它的渲染器照常只排行 */
  bar?: ReadingBar
}

/** 一份读数面板。空 sections 是合法值（「没有可报的」），前端渲染空态而非错误。 */
export interface ReadingPanel {
  title: string
  subtitle?: string
  sections?: ReadingSection[]
}

/** 分区里的行总数（含嵌套）；空面板 = 0。 */
export function panelRowCount(panel: ReadingPanel): number {
  return (panel.sections ?? []).reduce((n, s) => n + s.rows.length, 0)
}

/**
 * 边界收口：把**任意来源**的一个 `unknown`（节点属性 / 响应字段）收成可用的一格读数，
 * 认不出形状就 `undefined`。
 *
 * 只认**结构**（`rows` 是数组），不认业务词：产者多一行少一行都不需要改这里，
 * 而一个畸形载荷（`rows` 不是数组）在这里就被挡掉，不会到渲染时才炸。
 */
export function readingSectionOf(raw: unknown): ReadingSection | undefined {
  if (!raw || typeof raw !== 'object') return undefined
  const v = raw as Record<string, any>
  if (!Array.isArray(v.rows)) return undefined
  return v as ReadingSection
}

/** 有没有「需留意」的行——前端据此决定标题处的提示点，不解析值的语义。 */
export function panelHasWarn(panel: ReadingPanel): boolean {
  return (panel.sections ?? []).some((s) => s.rows.some((r) => r.tone === 'warn'))
}
