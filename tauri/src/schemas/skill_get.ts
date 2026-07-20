// Corresponding Backend: symbio/src/symbio_core/schemas/skill_get.rs
//
// `skill/get` 路由：获取指定 skill 的详情（含 body 预览）
// BUG-FR9：用于 SkillView 详情面板展示 SKILL.md body 全文。

export type SkillSource = 'workspace' | 'system' | 'external' | 'unknown'

/** skill/get - 获取指定 skill 详情 */
export namespace SkillGet {
  export interface Request {
    /** skill 名称 */
    name: string
    /** 可选：覆盖 workdir */
    workdir?: string | null
  }

  export interface Response {
    name: string
    description: string
    file_path: string
    source: SkillSource
    argument_hint?: string | null
    when_to_use?: string | null
    /** SKILL.md body 全文（已按 max_body_chars 截断） */
    body: string
    /** body 字符数 */
    body_chars: number
    /** body 是否被截断 */
    body_truncated: boolean
  }
}
