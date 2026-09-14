/**
 * 级联选项服务 —— 选项机制的唯一协议入口。
 *
 * 端点由**选项宿主**（session 插件）提供；前端只调用这一个标准端口，
 * 不含任何具体业务选项（工作目录 / 智能体 / 模型… 全部由后端下发）。
 *
 * 与后端对齐：symbio/src/symbio_core/schemas/options.rs
 */

import { callPlugin } from './plugin'
import { OPTIONS_LIST } from '@/constants/pluginPaths'
import type { OptionsRequest, OptionsResponse } from '@/schemas/options'
import { logger } from '@/utils/logger'

/**
 * 拉取选项列表。
 *
 * `parent` 缺省 = 根层（会话输入区的根选项）；非空 = 该节点的子项
 * （sub 类型懒加载，与 VDFS `vdfs/list` 的树懒加载同构）。
 */
export async function listOptions(
  sessionId?: string,
  parent?: string
): Promise<OptionsResponse> {
  try {
    const resp = await callPlugin<OptionsResponse, OptionsRequest>(
      OPTIONS_LIST,
      {
        session_id: sessionId || undefined,
        parent: parent || undefined,
      },
      undefined,
      sessionId ? { session_id: sessionId } : undefined
    )
    return resp ?? { nodes: [] }
  } catch (err) {
    logger.error('options-service', 'listOptions failed:', err)
    return { nodes: [] }
  }
}
