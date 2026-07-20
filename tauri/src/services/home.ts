/**
 * Home 插件前端服务
 *
 * **职责**：封装后端 home 插件的所有前端 API 调用。
 *
 * home 插件是 Symbio 的根插件，负责：
 * 1. 全局配置持久化（`<homedir>/config.yaml`）
 * 2. 路由分发与工具聚合
 * 3. 子插件（agent/model/mcp/skill/session/explorer）实例化与生命周期管理
 * 4. **系统目录 (homedir) 切换**：调用 `home/reload` 热重载子插件
 * 5. **工作区 (workdir) 切换**：调用 `work/*` 路由（也由 home 插件处理）
 *
 * 因此本服务统一提供：
 * - `getHomedirInfo` / `switchHomedir`：homedir 信息与切换
 * - `getWorkspacePath` / `setWorkspacePath`：当前 workdir 与切换
 *
 * 对应后端路由：
 * - `home/get_homedir` → `getHomedirInfo`
 * - `home/reload` → `switchHomedir`
 * - `work/get_workspace` → `getWorkspacePath`
 * - `work/set_workspace` → `setWorkspacePath`
 *
 * 对应后端代码：symbio/src/plugins/home/plugin.rs
 * 对应 schema：tauri/src/schemas/{home_reload,work_get_workspace,work_set_workspace}.ts
 */
import { callPlugin, setGlobalWorkdir } from './plugin'
import type { Response as ReloadResponse } from '../schemas/home_reload'
import type { Response as WorkGetWorkspaceResponse } from '../schemas/work_get_workspace'
import type { Response as WorkSetWorkspaceResponse } from '../schemas/work_set_workspace'
import { logger } from '@/utils/logger'

// =====================================================================
// 系统目录 (homedir) 管理
// =====================================================================

/**
 * homedir 信息
 */
export interface HomedirInfo {
  /** 当前 homedir 绝对路径 */
  homedir: string
  /** bootstrap 文件位置（位于用户主目录下） */
  bootstrap_path: string
}

/**
 * 获取当前 homedir
 *
 * 调用 `home/get_homedir` 路由。后端流程：从 [`HomedirRegistry`] 读取当前 homedir 与 bootstrap 位置。
 *
 * @returns 当前 homedir 信息。后端未启动 / 路由未注册时返回空字符串。
 */
export async function getHomedirInfo(): Promise<HomedirInfo> {
  try {
    const resp = await callPlugin<HomedirInfo>('home/get_homedir', {})
    if (resp && resp.homedir) {
      return resp
    }
    return { homedir: '', bootstrap_path: '' }
  } catch (err) {
    logger.error('home-service', 'getHomedirInfo failed:', err)
    return { homedir: '', bootstrap_path: '' }
  }
}

/**
 * 切换 homedir（热重载）
 *
 * 调用 `home/reload` 路由。后端流程：
 * 1. 持久化新 homedir 到 `~/.symbio_bootstrap` 并更新内存
 * 2. 旧 config 写回旧 homedir（如有切换）
 * 3. 从新 homedir 重新读 config
 * 4. 清空 `instances` map
 * 5. 重建 worker composite
 * 6. 异步恢复 workdir
 *
 * **前端责任**（调用本函数前/后）：
 * 1. 关闭所有活跃 chat 会话（disconnectPlugin）
 * 2. 调用成功后重新拉取数据（refreshData）
 *
 * @param homedir 目标 homedir（绝对路径或 `~` 前缀）
 * @returns 切换结果；失败时返回 null
 */
export async function switchHomedir(homedir: string): Promise<ReloadResponse | null> {
  try {
    const resp = await callPlugin<ReloadResponse>('home/reload', { homedir })
    return resp
  } catch (err) {
    logger.error('home-service', `switchHomedir(${homedir}) failed:`, err)
    return null
  }
}

// =====================================================================
// 工作区 (workdir) 管理
// =====================================================================

/**
 * 获取当前工作区路径详情
 *
 * 调用 `work/get_workspace` 路由。返回 workdir、expanded_path、recent_workspaces 等。
 * 副作用：若返回了有效路径，会自动更新全局 workdir（`setGlobalWorkdir`）。
 */
export async function getWorkspacePath(): Promise<WorkGetWorkspaceResponse> {
  const result = await callPlugin<WorkGetWorkspaceResponse>('work/get_workspace', {})
  if (result) {
    const path = result.workdir || result.expanded_path
    if (path && path !== '~/projects' && !path.endsWith('/projects')) {
      setGlobalWorkdir(path)
    }
  }
  return result
}

/**
 * 设置当前工作区路径
 *
 * 调用 `work/set_workspace` 路由。副作用：更新全局 workdir。
 */
export async function setWorkspacePath(path: string): Promise<WorkSetWorkspaceResponse> {
  const result = await callPlugin<WorkSetWorkspaceResponse>('work/set_workspace', { path })
  setGlobalWorkdir(path)
  return result
}
