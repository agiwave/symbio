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
 * 5. **工作区 (workdir) 切换**：调用 `home/set_workspace` · `home/get_workspace` 路由
 *    （同样由 home 插件处理——前缀必须是目录名，这几条臂的 owner 就是 home）
 *
 * 因此本服务统一提供：
 * - `getHomedirInfo` / `switchHomedir`：homedir 信息与切换
 * - `getWorkspacePath` / `setWorkspacePath`：当前 workdir 与切换
 *
 * 对应后端代码：crates/symbio-plugin-home/src/plugin.rs
 * 对应 schema：`schemas/home.ts`——四条臂的载荷各按后端模块名收在一个同名 namespace
 * 里。本模块**不再自持契约**（此前那三个「本模块私有契约」正是抄本），
 * 字段形状由 `protocol-mirror-audit` D 组逐字段看守。
 */
import { callPlugin, setLastWorkdir } from './plugin'
import type { home_get_homedir, home_get_workspace, home_reload, home_set_workspace } from '@/schemas/home'
import { withFallback } from './fallback'
import {
  ROUTE_HOME_GET_HOMEDIR,
  ROUTE_HOME_GET_WORKSPACE,
  ROUTE_HOME_RELOAD,
  ROUTE_HOME_SET_WORKSPACE,
} from '@/constants/routes.gen'

// =====================================================================
// 系统目录 (homedir) 管理
// =====================================================================

/**
 * 获取当前 homedir
 *
 * 调用 `home/get_homedir` 路由。后端流程：从 [`HomedirRegistry`] 读取当前 homedir 与 bootstrap 位置。
 *
 * @returns 当前 homedir 信息。后端未启动 / 路由未注册时返回空字符串。
 */
export async function getHomedirInfo(): Promise<home_get_homedir.Response> {
  const empty = (): home_get_homedir.Response => ({ homedir: '', bootstrap_path: '' })
  return withFallback(
    async () => {
      const resp = await callPlugin<home_get_homedir.Response>(ROUTE_HOME_GET_HOMEDIR, {})
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
 * 1. 校验目标目录并在目标 homedir 构造候选配置与 worker composite
 * 2. 把当前 config 持久化到旧 homedir（如有切换）
 * 3. 全部成功后持久化 bootstrap，并一次性替换 config / instances
 * 4. 异步恢复 workdir
 *
 * **前端责任**（调用本函数前/后）：
 * 1. 关闭所有活跃 chat 会话（disconnectPlugin）
 * 2. 调用成功后重新拉取数据（refreshData）
 *
 * @param homedir 目标 homedir（绝对路径或 `~` 前缀）
 * @param opts.forceNative 仅控制面提示；不能越过已选 Server HTTP。切换本地目标时应先
 *        持久化并重载 outbound，再调用本函数，失败则由调用方恢复原目标与传输
 * @returns 切换结果；失败时抛出，调用方不得报告成功
 */
export async function switchHomedir(
  homedir: string,
  opts?: { forceNative?: boolean }
): Promise<home_reload.Response> {
  return callPlugin<home_reload.Response>(
    ROUTE_HOME_RELOAD,
    { homedir },
    undefined,
    opts?.forceNative ? { forceNative: true } : undefined
  )
}

// =====================================================================
// 工作区 (workdir) 管理
// =====================================================================

/**
 * 后端**还没配过 workdir** 时返回的占位值——它不代表真实目录。
 *
 * 前端据此**不把它记成最近使用目录**，否则新建会话会默认落到一个不存在的路径。
 */
const DEFAULT_WORKSPACE = '~/projects'

/**
 * 获取当前工作区路径详情
 *
 * 调用 `home/get_workspace` 路由。返回 workdir、expanded_path、recent_workspaces 等。
 * 副作用：记录为最近使用目录（`setLastWorkdir`，仅作新建会话默认）。
 */
export async function getWorkspacePath(): Promise<home_get_workspace.Response> {
  const result = await callPlugin<home_get_workspace.Response>(ROUTE_HOME_GET_WORKSPACE, {})
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
 * 调用 `home/set_workspace` 路由。副作用：记录为最近使用目录。
 */
export async function setWorkspacePath(path: string): Promise<home_set_workspace.Response> {
  const result = await callPlugin<home_set_workspace.Response>(ROUTE_HOME_SET_WORKSPACE, { path })
  setLastWorkdir(path)
  return result
}
