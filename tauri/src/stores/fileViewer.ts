/**
 * 文件查看器 Store
 *
 * 资源浏览器选中文件时，调用 show(path, workdir) 打开全屏覆盖层；
 * close() 关闭。
 *
 * 状态与 explorer store 解耦：explorer 只关心"哪个文件被选中"，
 * viewer 关心"当前是否打开了文件查看器、显示哪个文件"。
 */

import { defineStore } from 'pinia'
import { ref } from 'vue'

export const useFileViewerStore = defineStore('fileViewer', () => {
  const isOpen = ref(false)
  /** 相对于 workdir 的路径 */
  const path = ref<string | null>(null)
  /** 当时的 workdir（绝对路径），用于后端 read 时定位 */
  const workdir = ref<string | null>(null)
  /** 文件名（用于显示） */
  const name = ref<string | null>(null)

  function show(p: string, wd: string | null) {
    path.value = p
    workdir.value = wd
    name.value = p.replace(/\\/g, '/').split('/').filter(Boolean).pop() || p
    isOpen.value = true
  }

  function close() {
    isOpen.value = false
    path.value = null
    workdir.value = null
    name.value = null
  }

  return { isOpen, path, workdir, name, show, close }
})
