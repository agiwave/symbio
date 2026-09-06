/**
 * Explorer 存储 —— 文件查看器的写回通道
 *
 * 目录树浏览已机制化（EntityTree + 会话容器实体页），本 store 仅保留
 * 文件查看器（FileViewerOverlay）的保存通道；workdir 经 services/plugin
 * 的 lastWorkdir 侧信道随请求下发。
 */
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { callPlugin } from '@/services/plugin'
import { logger } from '@/utils/logger'

export const useExplorerStore = defineStore('explorer', () => {
  const error = ref('')

  /** 保存文件（explorer/write；成功返回 true，失败信息在 error） */
  async function saveFile(path: string, content: string): Promise<boolean> {
    error.value = ''
    try {
      await callPlugin('explorer/write', { path, content })
      return true
    } catch (e) {
      error.value = e instanceof Error ? e.message : String(e)
      logger.error('explorer', 'Failed to save file', e)
      return false
    }
  }

  return { error, saveFile }
})
