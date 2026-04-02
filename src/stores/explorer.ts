/**
 * 资源浏览器存储
 *
 * 通过后端 explorer 插件管理文件/目录浏览
 */

import { defineStore } from 'pinia'
import { ref, computed, shallowRef, triggerRef } from 'vue'
import { callPlugin } from '@/services/plugin'

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

  // 辅助函数：更新文件树
  function updateFileTree(updater: (map: Map<string, FileItem>) => void) {
    const newMap = new Map(fileTree.value)
    updater(newMap)
    fileTree.value = newMap
    triggerRef(fileTree)
  }

  // 计算属性：根目录项（第一层级的所有项）
  const rootItems = computed(() => {
    const items: FileItem[] = []

    // 遍历 fileTree，找出第一层级的项
    fileTree.value.forEach((item, key) => {
      // 根目录项：key 不包含 '/' 字符
      if (!key.includes('/')) {
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
    const children: FileItem[] = []
    const prefix = parentPath ? parentPath + '/' : ''
    const parentDepth = parentPath ? parentPath.split('/').length : 0
    
    fileTree.value.forEach((item, key) => {
      // 检查是否是直接子项
      if (key.startsWith(prefix) && key !== parentPath) {
        const keyDepth = key.split('/').length
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

      currentPath.value = result.path || ''

      // 更新文件树
      if (result.items && Array.isArray(result.items)) {

        updateFileTree((map) => {
          if (recursive) {
            // 递归加载时，扁平化所有项
            function flattenItems(items: FileItem[]): FileItem[] {
              let flat: FileItem[] = []
              for (const item of items) {
                flat.push(item)
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
              map.set(item.path, item)
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

    const item = fileTree.value.get(path)
    if (!item) {
      // 尝试从后端获取
      await getItem(path)
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

  return {
    // State
    fileTree,
    currentPath,
    selectedPath,
    fileContent,
    initialized,
    loading,
    error,

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

    // Helper
    getChildren,
  }
})
