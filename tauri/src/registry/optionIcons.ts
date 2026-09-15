/**
 * 选项图标注册表（前端纯 UI 资产）
 *
 * 与 `registry/vdfsIcons.ts` 的图标映射同一分层定位：**后端下发
 * 图标名，前端负责把名字映射为具体视觉**。后端不参与下发 emoji/SVG。
 *
 * 未登记的图标名回落默认图标；选项节点仍可正常渲染（只是图标为通用形），
 * 因此新增贡献方无需改动本文件。
 */

/** 图标名 → 视觉（emoji，与既有会话页选项行视觉语言一致） */
const OPTION_ICONS: Record<string, string> = {
  folder: '📁',
  agent: '🎭',
  model: '🧠',
  risk: '🎯',
  'run-mode': '💬',
  heartbeat: '⏰',
  play: '▶',
}

/** 未登记图标名的回落 */
export const DEFAULT_OPTION_ICON = '⚙'

/** 解析图标名（未知回落默认图标） */
export function optionIcon(name?: string): string {
  if (!name) return DEFAULT_OPTION_ICON
  return OPTION_ICONS[name] ?? DEFAULT_OPTION_ICON
}
