// Corresponding Backend: symbio/src/symbio_core/schemas/skill_list.rs
//
// `skill/list` 路由：列出所有已加载的 skill（按来源分类）
//
// Skill 来源（source）分类：
// - `workspace`: 工作区级（{workdir}/.symbio/skills）
// - `system`   : 系统级（~/.symbio/plugins/skills）
// - `external` : 第三方（.qwen / .sixth / .qoder 等）
// - `unknown`  : 未匹配到

export type SkillSource = 'workspace' | 'system' | 'external' | 'unknown'

export interface SkillInfo {
  name: string
  description: string
  file_path: string
  source: SkillSource
  argument_hint?: string | null
  when_to_use?: string | null
}

/** skill/list - 列出所有已加载 skill */
export namespace SkillList {
  export interface Request {
    /** 可选：覆盖 workdir */
    workdir?: string | null
  }
  export interface Response {
    skills: SkillInfo[]
  }
}
