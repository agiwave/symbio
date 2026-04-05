/**
 * 文件监听服务
 * 
 * 监听后端发出的文件/目录变化事件，通知前端更新
 */

import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'

export interface BrowserFileChangeEvent {
  path: string
  kind: string
  timestamp: number
}

/**
 * 监听目录变化
 */
export async function onDirChanged(callback: (event: BrowserFileChangeEvent) => void) {
  return listen<BrowserFileChangeEvent>('browser/dir_changed', (event) => {
    callback(event.payload)
  })
}

/**
 * 监听文件变化
 */
export async function onFileChanged(callback: (event: BrowserFileChangeEvent) => void) {
  return listen<BrowserFileChangeEvent>('browser/file_changed', (event) => {
    callback(event.payload)
  })
}

/**
 * 启动文件监听（通过 Explorer 插件）
 */
export async function startWatching(): Promise<void> {
  await invoke('invoke', {
    path: 'explorer',
    input: { action: 'start_watch' }
  })
}

/**
 * 停止文件监听（通过 Explorer 插件）
 */
export async function stopWatching(): Promise<void> {
  await invoke('invoke', {
    path: 'explorer',
    input: { action: 'stop_watch' }
  })
}
