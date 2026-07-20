/**
 * 资源浏览器存储
 *
 * 通过后端 explorer 插件管理文件/目录浏览
 * 文件变化事件由中央 event bus 推送（kind: 'explorer'），
 * 取代了旧的 connectPlugin 直连模式。
 */

import { defineStore } from 'pinia'
import { ref, computed, shallowRef, triggerRef } from 'vue'
import { callPlugin } from '@/services/plugin'
import { subscribe as busSubscribe, type BusEvent, KIND_EXPLORER } from '@/services/eventBus'
import { FileItem, Request as ListRequest, Response as ListResponse } from '@/schemas/explorer_list'
import { Request as ReadRequest, Response as ReadResponse } from '@/schemas/explorer_read'
import { Request as WriteRequest, Response as WriteResponse } from '@/schemas/explorer_write'
import { ExplorerEventType } from '@/schemas/explorer_event'
import { logger } from '@/utils/logger'

/** 规范化路径：统一使用正斜杠，确保跨平台一致 */
function normalizePath(p: string): string {
  return p.replace(/\\/g, '/')
}

// FileItem moved to protocol

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

  // 已加载过的目录（避免重复请求 + 让"折叠后再次展开"瞬时响应）
  const loadedDirs = shallowRef(new Set<string>())

  // 展开状态：存到 store 里，跨 FileTreeNode 重建也能保留
  const expandedDirs = shallowRef(new Set<string>())

  // 正在加载的子目录（用于节点上的小 spinner，不影响整棵树的渲染）
  const loadingDirs = shallowRef(new Set<string>())

  // 监听状态（通过 bus 订阅）
  const isWatching = ref(false)
  let unsubscribeBus: (() => void) | null = null

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
      logger.error('explorer', 'init failed', e)
    }
  }

  // 加载目录
  // - 根目录（path == ''）：触发全局 loading（让整棵树 placeholder）
  // - 子目录：只标记 loadingDirs[path]，不污染全局 loading，
  //   避免 FileTreeNode 被 v-if 整体卸载导致展开状态丢失
  async function loadDirectory(path: string, recursive = false) {
    const isRoot = !path
    const normPath = normalizePath(path || '')

    if (isRoot) {
      loading.value = true
    } else {
      const s = new Set(loadingDirs.value)
      s.add(normPath)
      loadingDirs.value = s
      triggerRef(loadingDirs)
    }
    error.value = null

    try {
      // path 应该是相对于工作区的路径，空字符串表示工作区根
      const result = await callPlugin<ListResponse, ListRequest>('explorer/list', {
        path: path || undefined,
        recursive
      })

      if (!result) {
        logger.warn('explorer', 'empty result from backend')
        return
      }

      // 更新当前路径（保持为相对路径）
      if (isRoot) {
        currentPath.value = ''
      }

      // 标记本目录已加载（用于 expandDirectory 跳过重复请求）
      const newLoaded = new Set(loadedDirs.value)
      newLoaded.add(normPath)
      loadedDirs.value = newLoaded
      triggerRef(loadedDirs)

      // 更新文件树
      if (result.items && Array.isArray(result.items)) {

        updateFileTree((map) => {
          // 规范化当前目录路径
          const normCurrentPath = normalizePath(path || '')
          const currentDepth = normCurrentPath ? normCurrentPath.split('/').length : 0
          const prefix = normCurrentPath ? normCurrentPath + '/' : ''

          // 先清除当前目录下的旧项目（直接子项）
          const keysToRemove: string[] = []
          map.forEach((_item, key) => {
            const normKey = normalizePath(key)
            // 检查是否是当前目录的直接子项
            if (normCurrentPath === '') {
              // 根目录：深度为 1 的项
              if (normKey.split('/').length === 1) {
                keysToRemove.push(key)
              }
            } else {
              // 子目录：以 prefix 开头且深度 = 当前深度 + 1
              if (normKey.startsWith(prefix) && normKey.split('/').length === currentDepth + 1) {
                keysToRemove.push(key)
              }
            }
          })
          // 删除旧项目
          for (const key of keysToRemove) {
            map.delete(key)
          }

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
        logger.warn('explorer', 'no items in result')
      }
    } catch (e) {
      error.value = e instanceof Error ? e.message : '加载目录失败'
      logger.error('explorer', 'loadDirectory failed', e)
    } finally {
      if (isRoot) {
        loading.value = false
      } else {
        const s = new Set(loadingDirs.value)
        s.delete(normPath)
        loadingDirs.value = s
        triggerRef(loadingDirs)
      }
    }
  }

  // 获取文件/目录详情
  async function getItem(path: string): Promise<FileItem | null> {
    try {
      const result = await callPlugin<FileItem, { path: string }>('explorer/get', { path })
      return result
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      error.value = `获取 ${path} 失败: ${msg}`
      logger.error('explorer', 'Failed to get item', e)
      return null
    }
  }

  async function readFile(path: string): Promise<string | null> {
    try {
      const result = await callPlugin<ReadResponse, ReadRequest>('explorer/read', { path })
      fileContent.value = result.content
      return result.content
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      error.value = `读取 ${path} 失败: ${msg}`
      logger.error('explorer', 'Failed to read file', e)
      return null
    }
  }

  // 选择文件/目录
  // 只更新选中状态 + 必要时从后端 fetch item 元数据；
  // 文件内容由 FileViewerOverlay 等调用方按需读取，避免重复 IO。
  async function selectItem(path: string) {
    selectedPath.value = path

    let item: FileItem | undefined | null = fileTree.value.get(path)
    if (!item) {
      // 尝试从后端获取
      item = await getItem(path)
    }

    if (!item || item.is_dir) {
      fileContent.value = null
    }
  }

  // 展开/折叠状态判断
  function isExpanded(path: string): boolean {
    return expandedDirs.value.has(normalizePath(path))
  }

  function isDirLoading(path: string): boolean {
    return loadingDirs.value.has(normalizePath(path))
  }

  // 切换展开：自动触发懒加载
  // 状态保存在 store 里，跨 FileTreeNode 重建仍保留
  function toggleExpand(path: string) {
    const norm = normalizePath(path)
    const next = new Set(expandedDirs.value)
    let nowExpanded: boolean
    if (next.has(norm)) {
      next.delete(norm)
      nowExpanded = false
    } else {
      next.add(norm)
      nowExpanded = true
    }
    expandedDirs.value = next
    triggerRef(expandedDirs)

    // 展开 + 未加载 → 触发懒加载
    if (nowExpanded && !loadedDirs.value.has(norm)) {
      loadDirectory(path, false)
    }
  }

  // 显式展开（不切换）
  function expand(path: string) {
    const norm = normalizePath(path)
    if (expandedDirs.value.has(norm)) return
    const next = new Set(expandedDirs.value)
    next.add(norm)
    expandedDirs.value = next
    triggerRef(expandedDirs)
    if (!loadedDirs.value.has(norm)) {
      loadDirectory(path, false)
    }
  }

  // 显式折叠
  function collapse(path: string) {
    const norm = normalizePath(path)
    if (!expandedDirs.value.has(norm)) return
    const next = new Set(expandedDirs.value)
    next.delete(norm)
    expandedDirs.value = next
    triggerRef(expandedDirs)
  }

  // 兼容旧 API：仅当未加载时加载（不切换状态）
  async function expandDirectory(path: string) {
    const norm = normalizePath(path)
    if (loadedDirs.value.has(norm)) return
    await loadDirectory(path, false)
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
      
      await callPlugin<WriteResponse, WriteRequest>('explorer/write', {
        path,
        content
      })

      // 只要 callPlugin 没有抛出异常，说明保存成功
      fileContent.value = content
      return true
    } catch (e) {
      const errMsg = e instanceof Error ? e.message : '保存失败'
      error.value = errMsg
      logger.error('explorer', 'Failed to save file', e)
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
    loadedDirs.value = new Set()
    expandedDirs.value = new Set()
    loadingDirs.value = new Set()
    stopWatching()
  }

  // 启动文件监听（订阅中央事件总线）
  async function startWatching() {
    if (isWatching.value || unsubscribeBus) return

    try {
      unsubscribeBus = busSubscribe(
        { kind: KIND_EXPLORER },
        handleBusEvent
      )
      isWatching.value = true
    } catch (e) {
      logger.error('explorer', 'Failed to start watching', e)
    }
  }

  // 停止文件监听
  function stopWatching() {
    if (unsubscribeBus) {
      unsubscribeBus()
      unsubscribeBus = null
    }
    isWatching.value = false
  }

  // 将 bus 事件转换为 explorer 内部事件，再复用原处理逻辑
  function handleBusEvent(busEvent: BusEvent) {
    if (!busEvent.data) return
    const eventData = busEvent.data.data as { type?: string; data?: unknown }
    if (!eventData || !eventData.type) return

    // bus data 形如 { type, data }，直接喂给 handleConnectEvent
    handleConnectEvent({ type: eventData.type, data: eventData.data })
  }

  // 处理 explorer 事件（来自 bus）
  async function handleConnectEvent(event: { type: string; data?: unknown }) {
    const data = event.data as { path?: string; kind?: string; message?: string } | undefined
    switch (event.type) {
      case 'connected':
        break

      case ExplorerEventType.WatchStarted:
        break

      case ExplorerEventType.WatchStopped:
        isWatching.value = false
        break

      case ExplorerEventType.DirChanged:
      case ExplorerEventType.FileChanged: {
        // 文件/目录变化，根据操作类型处理
        // 后端发送的 kind 格式：Remove(...), Create(...), Modify(...) 等
        const changedPath = data?.path
        const kind = data?.kind || ''

        if (changedPath) {
          const normChangedPath = normalizePath(changedPath)

          // 判断是否是删除操作
          const isRemove = kind.toLowerCase().includes('remove') ||
                          kind.toLowerCase().includes('delete')

          if (isRemove) {
            // 删除操作：直接从 fileTree 中移除
            updateFileTree((map) => {
              // 删除该项及其所有子项
              const keysToRemove: string[] = []
              map.forEach((_item, key) => {
                const normKey = normalizePath(key)
                if (normKey === normChangedPath || normKey.startsWith(normChangedPath + '/')) {
                  keysToRemove.push(key)
                }
              })
              for (const key of keysToRemove) {
                map.delete(key)
              }
            })

            // 如果删除的是当前选中的文件，清除选中状态
            if (selectedPath.value) {
              const normSelected = normalizePath(selectedPath.value)
              if (normSelected === normChangedPath || normSelected.startsWith(normChangedPath + '/')) {
                selectedPath.value = null
                fileContent.value = null
              }
            }
          } else {
            // 创建或修改操作：刷新父目录
            // 检查是否是当前选中的文件
            const isCurrentFile = selectedPath.value === changedPath ||
                                  normalizePath(selectedPath.value || '').endsWith(normChangedPath)

            if (isCurrentFile && selectedPath.value) {
              // 重新读取当前文件内容
              await selectItem(selectedPath.value)
            }

            // 刷新父目录
            const parentPath = normChangedPath.substring(0, normChangedPath.lastIndexOf('/')) || ''
            await loadDirectory(parentPath, false)
          }
        } else {
          await refresh()
        }
        break
      }

      case 'error':
        error.value = data?.message || '连接错误'
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

    // Computed
    rootItems,

    // Actions
    init,
    loadDirectory,
    getItem,
    readFile,
    selectItem,
    isExpanded,
    isDirLoading,
    toggleExpand,
    expand,
    collapse,
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
