/**
 * 资源浏览器存储
 *
 * 通过后端 explorer 插件管理文件/目录浏览
 * 使用 connect 机制实现实时文件变化监听
 */

import { defineStore } from 'pinia'
import { ref, computed, shallowRef, triggerRef } from 'vue'
import { callPlugin, connectPlugin, Connection, ConnectEvent } from '@/services/plugin'

/** 规范化路径：统一使用正斜杠，确保跨平台一致 */
function normalizePath(p: string): string {
  return p.replace(/\\/g, '/')
}

export interface FileItem {
  name: string
  path: string
  is_dir: boolean
  size?: number
  children?: FileItem[]
}

export const useExplorerStore = defineStore('explorer', () => {
  // 文件树 - 使用 shallowRef + Map，手动触发更新
  const fileTree = shallowRef(new Map<string, FileItem>())

  // 当前路径
  const currentPath = ref<string>('')

  // 选中的文件/目录
  const selectedPath = ref<string | null>(null)

  // 当前文件内容（如果是文件）
  const fileContent = ref<string | null>(null)

  // 是否已初始化
  const initialized = ref(false)

  // 加载状态
  const loading = ref(false)

  // 错误信息
  const error = ref<string | null>(null)

  // 连接状态
  const connection = ref<Connection | null>(null)
  const isWatching = ref(false)

  // 辅助函数：更新文件树
  function updateFileTree(updater: (map: Map<string, FileItem>) => void) {
    const newMap = new Map(fileTree.value)
    updater(newMap)
    fileTree.value = newMap
    triggerRef(fileTree)
  }

  // 计算属性：根目录项（最小深度的所有项）
  const rootItems = computed(() => {
    const items: FileItem[] = []

    // 找出所有项的最小深度
    let minDepth = Infinity
    fileTree.value.forEach((_item, key) => {
      const depth = normalizePath(key).split('/').length
      if (depth < minDepth) minDepth = depth
    })

    // 只取最小深度的项（即根目录层级）
    fileTree.value.forEach((item, key) => {
      const depth = normalizePath(key).split('/').length
      if (depth === minDepth) {
        items.push(item)
      }
    })

    // 按名称排序：目录在前，文件在后
    items.sort((a, b) => {
      if (a.is_dir && !b.is_dir) return -1
      if (!a.is_dir && b.is_dir) return 1
      return a.name.localeCompare(b.name)
    })


    return items
  })

  // 获取子项列表
  function getChildren(parentPath: string): FileItem[] {
    // 规范化路径
    const normParent = normalizePath(parentPath)
    const prefix = normParent ? normParent + '/' : ''
    const parentDepth = normParent ? normParent.split('/').length : 0

    const children: FileItem[] = []

    fileTree.value.forEach((item, key) => {
      const normKey = normalizePath(key)
      // 检查是否是直接子项
      if (normKey.startsWith(prefix) && normKey !== normParent) {
        const keyDepth = normKey.split('/').length
        // 只取直接子项（深度 = 父深度 + 1）
        if (keyDepth === parentDepth + 1) {
          children.push(item)
        }
      }
    })
    
    // 按名称排序：目录在前，文件在后
    children.sort((a, b) => {
      if (a.is_dir && !b.is_dir) return -1
      if (!a.is_dir && b.is_dir) return 1
      return a.name.localeCompare(b.name)
    })
    
    return children
  }

  // 初始化
  async function init() {
    if (initialized.value) return

    try {
      await loadDirectory('')
      initialized.value = true
    } catch (e) {
      error.value = e instanceof Error ? e.message : '初始化失败'
      console.error('[explorer] init failed:', e)
    }
  }

  // 加载目录
  async function loadDirectory(path: string, recursive = false) {
    loading.value = true
    error.value = null

    try {
      // path 应该是相对于工作区的路径，空字符串表示工作区根
      const result = await callPlugin<{ path: string; items: FileItem[] }>('explorer', {
        action: 'list',
        path: path || undefined,
        recursive
      })

      if (!result) {
        console.warn('[explorer] empty result from backend')
        loading.value = false
        return
      }

      // 更新当前路径（保持为相对路径）
      if (path === '') {
        currentPath.value = ''
      }

      // 更新文件树
      if (result.items && Array.isArray(result.items)) {

        updateFileTree((map) => {
          if (recursive) {
            // 递归加载时，扁平化所有项
            function flattenItems(items: FileItem[]): FileItem[] {
              let flat: FileItem[] = []
              for (const item of items) {
                const normalized = { ...item, path: normalizePath(item.path) }
                flat.push(normalized)
                if (item.children) {
                  flat = flat.concat(flattenItems(item.children))
                }
              }
              return flat
            }

            const flatItems = flattenItems(result.items)
            for (const item of flatItems) {
              map.set(item.path, item)
            }
          } else {
            // 非递归加载，只更新当前层级
            for (const item of result.items) {
              const normalized = { ...item, path: normalizePath(item.path) }
              map.set(normalized.path, normalized)
            }
          }
        })
        
      } else {
        console.warn('[explorer] no items in result')
      }
    } catch (e) {
      error.value = e instanceof Error ? e.message : '加载目录失败'
      console.error('[explorer] loadDirectory failed:', e)
    } finally {
      loading.value = false
    }
  }

  // 获取文件/目录详情
  async function getItem(path: string): Promise<FileItem | null> {
    try {
      const result = await callPlugin<FileItem>('explorer', {
        action: 'get',
        path
      })
      return result
    } catch (e) {
      console.error('Failed to get item:', e)
      return null
    }
  }

  // 读取文件内容
  async function readFile(path: string): Promise<string | null> {
    try {
      const result = await callPlugin<{ content: string; file_type: string }>('explorer', {
        action: 'read',
        path
      })
      fileContent.value = result.content
      return result.content
    } catch (e) {
      console.error('Failed to read file:', e)
      return null
    }
  }

  // 选择文件/目录
  async function selectItem(path: string) {
    selectedPath.value = path

    let item: FileItem | undefined | null = fileTree.value.get(path)
    if (!item) {
      // 尝试从后端获取
      item = await getItem(path)
    }

    // 如果是文件，读取内容
    if (item && !item.is_dir) {
      await readFile(path)
    } else {
      fileContent.value = null
    }
  }

  // 展开目录（懒加载）
  async function expandDirectory(path: string) {
    const item = fileTree.value.get(path)
    if (item && item.is_dir && !item.children) {
      // 懒加载子项
      await loadDirectory(path, false)
    }
  }

  // 刷新当前目录
  async function refresh() {
    await loadDirectory(currentPath.value)
  }

  // 标记是否正在保存（用于文件监听器跳过重载）
  let isSavingFile = false

  // 保存文件内容
  async function saveFile(path: string, content: string): Promise<boolean> {
    try {
      isSavingFile = true
      
      await callPlugin('explorer', {
        action: 'write',
        path,
        content
      })

      // 只要 callPlugin 没有抛出异常，说明保存成功
      fileContent.value = content
      return true
    } catch (e) {
      const errMsg = e instanceof Error ? e.message : '保存失败'
      error.value = errMsg
      console.error('[explorer] Failed to save file:', e)
      return false
    } finally {
      isSavingFile = false
    }
  }

  // 重置状态（切换工作区时调用）
  function reset() {
    fileTree.value = new Map()
    currentPath.value = ''
    selectedPath.value = null
    fileContent.value = null
    initialized.value = false
    loading.value = false
    error.value = null
    stopWatching()
  }

  // 启动文件监听（使用 connect 机制）
  async function startWatching() {
    if (isWatching.value || connection.value) return

    try {
      const conn = await connectPlugin('explorer', { action: 'watch' }, handleConnectEvent)
      connection.value = conn
      isWatching.value = true
      console.log('[explorer] Started watching via connect')
    } catch (e) {
      console.error('[explorer] Failed to start watching:', e)
    }
  }

  // 停止文件监听
  async function stopWatching() {
    if (connection.value) {
      try {
        const { closeConnection } = await import('@/services/plugin')
        await closeConnection(connection.value.connectionId)
        connection.value.unlisten()
        connection.value = null
        isWatching.value = false
        console.log('[explorer] Stopped watching')
      } catch (e) {
        console.error('[explorer] Failed to stop watching:', e)
      }
    }
  }

  // 处理连接事件
  async function handleConnectEvent(event: ConnectEvent) {
    switch (event.type) {
      case 'connected':
        break

      case 'watch_started':
        break

      case 'watch_stopped':
        isWatching.value = false
        break

      case 'browser/dir_changed':
      case 'browser/file_changed': {
        // 文件/目录变化，刷新当前视图
        // 注意：后端发送的是相对于工作区的路径
        const changedPath = (event.data as any)?.path
        if (changedPath) {
          // 检查是否是当前选中的文件
          const isCurrentFile = selectedPath.value === changedPath ||
                                normalizePath(selectedPath.value || '').endsWith(normalizePath(changedPath))

          if (isCurrentFile) {
            // 重新读取当前文件内容
            if (selectedPath.value) {
              await selectItem(selectedPath.value)
            }
          }

          // 刷新文件树
          await refresh()
        } else {
          await refresh()
        }
        break
      }

      case 'error':
        error.value = (event.data as any)?.message || '连接错误'
        break
    }
  }

  return {
    // State
    fileTree,
    currentPath,
    selectedPath,
    fileContent,
    initialized,
    loading,
    error,
    isSavingFile,
    isWatching,
    connection,

    // Computed
    rootItems,

    // Actions
    init,
    loadDirectory,
    getItem,
    readFile,
    selectItem,
    expandDirectory,
    refresh,
    saveFile,
    reset,
    startWatching,
    stopWatching,

    // Helper
    getChildren,
  }
})
