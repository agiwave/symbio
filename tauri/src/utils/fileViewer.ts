/**
 * File Viewer Window 工具
 *
 * 使用 Tauri WebviewWindow 打开新窗口显示文件。
 * 路由：/file-viewer
 * 协议：search params: ?path=...&workdir=...
 *
 * 同一文件路径复用同一窗口（不会打开多个）
 */

import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { emitTo } from '@tauri-apps/api/event'
import { logger } from './logger'

const FILE_WINDOW_LABEL = 'file-viewer'

export interface OpenFileViewerOptions {
  path: string
  workdir?: string
  /** 自定义窗口标题 */
  title?: string
  /** 自定义宽度 */
  width?: number
  /** 自定义高度 */
  height?: number
}

/**
 * 打开（或聚焦）文件查看窗口。
 * 通过唯一的 label 保证同一窗口复用。
 */
export async function openFileViewer(opts: OpenFileViewerOptions): Promise<void> {
  // 1. 查是否已存在
  const existing = await WebviewWindow.getByLabel(FILE_WINDOW_LABEL)
  if (existing) {
    // 已存在 -> 聚焦并通过事件传递新文件
    try {
      await existing.show()
      await existing.setFocus()
      await emitTo(FILE_WINDOW_LABEL, 'file-viewer:load-file', {
        path: opts.path,
        workdir: opts.workdir,
        title: opts.title
      })
    } catch (e) {
      logger.warn('fileViewer', '聚焦现有窗口失败', e)
    }
    return
  }

  // 2. 创建新窗口
  const url = new URL(window.location.href)
  // 构造目标 URL：同 hash router 的 /file-viewer，附加 query
  const target = new URL(url.origin + url.pathname)
  target.hash = `#/file-viewer?path=${encodeURIComponent(opts.path)}${opts.workdir ? `&workdir=${encodeURIComponent(opts.workdir)}` : ''}${opts.title ? `&title=${encodeURIComponent(opts.title)}` : ''}`

  const w = new WebviewWindow(FILE_WINDOW_LABEL, {
    url: target.toString(),
    title: opts.title || basename(opts.path),
    width: opts.width || 900,
    height: opts.height || 700,
    minWidth: 400,
    minHeight: 300,
    resizable: true,
    center: true
  })

  w.once('tauri://created', () => {
    // 窗口创建后立即推一次（URL 已经带 query，但事件推送更稳）
    setTimeout(() => {
      emitTo(FILE_WINDOW_LABEL, 'file-viewer:load-file', {
        path: opts.path,
        workdir: opts.workdir,
        title: opts.title
      }).catch(() => {})
    }, 80)
  })

  w.once('tauri://error', (e) => {
    logger.error('fileViewer', '创建窗口失败', e)
  })
}

function basename(p: string): string {
  if (!p) return '文件查看'
  return p.replace(/\\/g, '/').split('/').filter(Boolean).pop() || p
}
