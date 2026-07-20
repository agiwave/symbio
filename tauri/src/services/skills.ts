/**
 * Skill 管理服务
 *
 * 包装后端 `skill/*` 路由：
 * - list   : 列出所有已加载的 skill（按来源分类）
 * - get    : 获取指定 skill 的详情（含 body），BUG-FR9
 *
 * 对应的后端：symbio/src/plugins/skill/plugin.rs
 * 对应的 schema：tauri/src/schemas/skill_list.ts, skill_get.ts
 *
 * **注意**：skill 是**只读**视图，用户通过在 `skill_dirs` 中放置 SKILL.md 文件
 * 来添加 skill（系统级 `~/.symbio/plugins/skills` 或工作区级 `.symbio/skills`）。
 * 因此 SkillView 没有"新建/编辑"按钮，只展示和查看。
 */

import { callPlugin } from './plugin'
import { SkillList, type SkillInfo } from '../schemas/skill_list'
import { SkillGet } from '../schemas/skill_get'
import { logger } from '@/utils/logger'

const SKILL_PATH = 'skill'

/** 列出所有已加载的 skill（按来源分组排序） */
export async function listSkills(workdir?: string): Promise<SkillInfo[]> {
  try {
    const resp = await callPlugin<SkillList.Response>(
      `${SKILL_PATH}/list`,
      { workdir: workdir ?? null } satisfies SkillList.Request
    )
    return resp?.skills ?? []
  } catch (err) {
    logger.error('skills-service', 'listSkills failed:', err)
    return []
  }
}

/** BUG-FR9：获取单个 skill 的详情（含 body） */
export async function getSkill(
  name: string,
  workdir?: string
): Promise<SkillGet.Response | null> {
  try {
    const resp = await callPlugin<SkillGet.Response>(
      `${SKILL_PATH}/get`,
      { name, workdir: workdir ?? null } satisfies SkillGet.Request
    )
    return resp ?? null
  } catch (err) {
    logger.error('skills-service', `getSkill(${name}) failed:`, err)
    return null
  }
}

/** 按 source 字段分组 */
export function groupSkillsBySource(
  skills: SkillInfo[]
): Record<string, SkillInfo[]> {
  const groups: Record<string, SkillInfo[]> = {
    workspace: [],
    system: [],
    external: [],
    unknown: []
  }
  for (const s of skills) {
    const group = groups[s.source] ?? groups.unknown
    group.push(s)
  }
  return groups
}

/** source 字段的中文标签 */
export function sourceLabel(source: string): string {
  switch (source) {
    case 'workspace': return '工作区'
    case 'system': return '系统'
    case 'external': return '第三方'
    default: return '未知'
  }
}

