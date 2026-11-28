/**
 * 统一的路径取值原语（DetailField.pick）。
 *
 * 三端交付物（桌面启动器 / CLI serve / web）的 Vue 实现都跑在普通浏览器里，
 * 没有可用的本机原生目录对话框。目录字段统一交给调用方维护的 DirectoryPicker，
 * 由 Server 目录 API（`/api/v1/host/directories`）浏览**目标后端**的文件系统：
 * 桌面端后端就是本机 CLI serve，因此浏览到的是本机目录。
 *
 * 浏览器绝不使用 `<input type=file>` 伪装目录选择，也不会读取浏览器本机路径。
 * 文件内容字段由 `pack` 动作通过 `<input type="file">` 读取内容并以 base64 上抛，
 * 不需要绝对路径——两条路径各司其职。
 */
import { DETAIL_PICK_DIRECTORY, DETAIL_PICKS, type DetailPick } from '@/schemas/vdfs-form'

export interface BrowserPathPickRequest {
  kind: DetailPick
  initialPath?: string
}

export interface PickPathOptions {
  title?: string
  initialPath?: string
  /** 打开 DirectoryPicker 的回调；选择结果由组件事件返回。 */
  onBrowserPick?: (request: BrowserPathPickRequest) => void
}

/**
 * 取值入口；用户取消或取值不在闭集内返回 null。
 * 仅支持目录选择；文件路径字段需手动填写（浏览器拿不到绝对路径）。
 */
export async function pickPath(
  kind: DetailPick | undefined,
  options: PickPathOptions = {},
): Promise<string | null> {
  if (!kind || !DETAIL_PICKS.includes(kind)) return null

  if (kind !== DETAIL_PICK_DIRECTORY) {
    throw new Error('当前 Server 仅支持目录浏览，文件路径请手动填写')
  }
  if (!options.onBrowserPick) {
    throw new Error('目录选择必须通过 DirectoryPicker 完成')
  }
  options.onBrowserPick({ kind, initialPath: options.initialPath })
  return null
}
