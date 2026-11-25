/**
 * 统一的路径取值原语（DetailField.pick）。
 *
 * embedded + native 使用 Tauri dialog；browser 与 embedded + remote 都没有可用的
 * 本机目录对话框，必须把请求交给调用方维护的 DirectoryPicker，由 Server 目录 API
 * 浏览目标后端文件系统。浏览器绝不使用 <input type=file> 伪装目录选择，也不会读取
 * 浏览器本机路径。
 */
import { DETAIL_PICK_DIRECTORY, DETAIL_PICKS, type DetailPick } from '@/schemas/vdfs-form'
import { getOutboundConfig } from './plugin'
import { isBrowserEnvironment } from './systemLocation'

export interface BrowserPathPickRequest {
  kind: DetailPick
  initialPath?: string
}

export interface PickPathOptions {
  title?: string
  initialPath?: string
  /** Server 分支打开 DirectoryPicker 的回调；选择结果由组件事件返回。 */
  onBrowserPick?: (request: BrowserPathPickRequest) => void
}

/**
 * 取值入口；用户取消或取值不在闭集内返回 null。
 * Server 仅支持目录选择，文件路径字段需手动填写。
 */
export async function pickPath(
  kind: DetailPick | undefined,
  options: PickPathOptions = {},
): Promise<string | null> {
  if (!kind || !DETAIL_PICKS.includes(kind)) return null

  if (isBrowserEnvironment() || getOutboundConfig().protocol === 'http') {
    if (kind !== DETAIL_PICK_DIRECTORY) {
      throw new Error('当前 Server 仅支持目录浏览，文件路径请手动填写')
    }
    if (!options.onBrowserPick) {
      throw new Error('浏览器目录选择必须通过 DirectoryPicker 完成')
    }
    options.onBrowserPick({ kind, initialPath: options.initialPath })
    return null
  }

  // 动态导入：普通浏览器构建路径不会静态初始化原生 dialog 能力。
  const { open } = await import('@tauri-apps/plugin-dialog')
  const directory = kind === DETAIL_PICK_DIRECTORY
  const picked = await open({
    directory,
    multiple: false,
    title: options.title ?? (directory ? '选择目录' : '选择文件'),
  })
  if (typeof picked === 'string') return picked
  if (Array.isArray(picked) && typeof picked[0] === 'string') return picked[0]
  return null
}
