/**
 * Home 插件前端服务
 *
 * **职责**：封装后端 home 插件的所有前端 API 调用。
 *
 * home 插件是 Symbio 的根插件，负责：
 * 1. 自身配置持久化（`<homedir>/PLUGIN.yml`：工作区 / 最近记录）
 * 2. 路由分发与工具聚合
 * 3. 容器 `composite` 的构造（子插件由它扫描 `<homedir>/*` 得到：一层目录 = 一个插件）
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
 * 对应 schema：tauri/src/schemas/home_reload.ts（work_get/set_workspace 已内联）
 */
import { callPlugin, setLastWorkdir } from './plugin'
import type { Response as ReloadResponse } from '../schemas/home_reload'
import { withFallback } from './fallback'
import {
  DEFAULT_WORKSPACE,
  HOME_GET_HOMEDIR,
  HOME_RELOAD,
  WORK_GET_WORKSPACE,
  WORK_SET_WORKSPACE,
} from '@/constants/pluginPaths'

// 原 schemas/work_get_workspace（仅本模块使用，内联）
export interface WorkGetWorkspaceResponse {
  workdir: string;
  expanded_path: string;
  recent_workspaces: string[];
}

// 原 schemas/work_set_workspace（仅本模块使用，内联）
export interface WorkSetWorkspaceResponse {
  workdir: string;
  expanded_path: string;
  recent_workspaces: string[];
  status: string;
}

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
  const empty = (): HomedirInfo => ({ homedir: '', bootstrap_path: '' })
  return withFallback(
    async () => {
      const resp = await callPlugin<HomedirInfo>(HOME_GET_HOMEDIR, {})
      return resp?.homedir ? resp : empty()
    },
    empty,
    { tag: 'home-service', what: 'getHomedirInfo failed' }
  )
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
 * @param opts.forceNative 是否强制走本机原生传输（切换系统目录时必须为 true，
 *        否则当前出站协议若指向远端会把 reload 路由到远端而失败）
 * @returns 切换结果；失败时返回 null
 */
export async function switchHomedir(
  homedir: string,
  opts?: { forceNative?: boolean }
): Promise<ReloadResponse | null> {
  return withFallback(
    () =>
      callPlugin<ReloadResponse>(
        HOME_RELOAD,
        { homedir },
        undefined,
        opts?.forceNative ? { forceNative: true } : undefined
      ),
    () => null,
    { tag: 'home-service', what: `switchHomedir(${homedir}) failed` }
  )
}

// =====================================================================
// 工作区 (workdir) 管理
// =====================================================================

/**
 * 获取当前工作区路径详情
 *
 * 调用 `work/get_workspace` 路由。返回 workdir、expanded_path、recent_workspaces 等。
 * 副作用：记录为最近使用目录（`setLastWorkdir`，仅作新建会话默认）。
 */
export async function getWorkspacePath(): Promise<WorkGetWorkspaceResponse> {
  const result = await callPlugin<WorkGetWorkspaceResponse>(WORK_GET_WORKSPACE, {})
  if (result) {
    const path = result.workdir || result.expanded_path
    if (path && path !== DEFAULT_WORKSPACE && !path.endsWith('/projects')) {
      setLastWorkdir(path)
    }
  }
  return result
}

/**
 * 设置当前工作区路径
 *
 * 调用 `work/set_workspace` 路由。副作用：记录为最近使用目录。
 */
export async function setWorkspacePath(path: string): Promise<WorkSetWorkspaceResponse> {
  const result = await callPlugin<WorkSetWorkspaceResponse>(WORK_SET_WORKSPACE, { path })
  setLastWorkdir(path)
  return result
}
