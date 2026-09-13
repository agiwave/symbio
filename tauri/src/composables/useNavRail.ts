/**
 * 应用外壳导航 —— `.vdfs` 驱动的唯一实现（MainLayout 消费）
 *
 * 与三栏页面规范（docs/design/vdfs-frontend.md）第 2 条一一对应：左栏列表来自
 * `.vdfs` 虚拟根，每项含 名字（mount）/ 标题（label）/ 图标 / 描述 /
 * 可接受的新建元素类型（`new_types`）。**前端零硬编码类型清单**：新增一类资源
 * = 后端注册一个 `VdfsProvider`，导航项自动出现。
 *
 * S5 起本文件是导航的**唯一**实现：统一实体页与其注册表
 * （`useEntityProviders`）已随实体页一并下线，应用外壳只认 `.vdfs` 挂载点。
 *
 * 实时：订阅 `vdfs` 事件总线，任一挂载点发生变更即重拉清单（**非轮询**）。
 */

import { computed, onBeforeUnmount, shallowRef } from 'vue'
import { useRoute } from 'vue-router'
import { fetchMounts } from '@/services/vdfs'
import { subscribe } from '@/services/eventBus'
import { VFDS_EVENT_KIND, mountNavVisible, type VdfsMountInfo } from '@/schemas/vdfs'
import { mountIconOf } from '@/registry/vdfsTypes'
import type { NavRailItem } from '@/components/common/NavRail.vue'

/**
 * 机制内路由约定：VDFS 挂载名 → 顶层导航路由目标。
 *
 * 全面 VDFS 化（S4）后全部资源（session / model / agent / skill / mcp / setting）
 * 都是 `.vdfs` 下的一个挂载点，故目标恒为 `/vdfs/{mount}`；首页 `/` 重定向至
 * `/vdfs/session`。本函数不持有任何类型特判，新增资源无需改动。
 */
export function navTargetOf(mount: string): string {
  return `/vdfs/${mount}`
}

/**
 * `.vdfs` 虚拟根内容（挂载点清单）—— 外壳导航的**唯一来源**（模块级单例）。
 *
 * S4 起外壳导航改由 `.vdfs` 驱动；S5 下线实体页后，VDFS 挂载点清单是前端
 * 唯一的一份「资源类别」来源（实体注册表与其页面已移除）。
 */
const mounts = shallowRef<VdfsMountInfo[]>([])
let state: 'idle' | 'loading' | 'loaded' = 'idle'
let loadingPromise: Promise<void> | null = null

/** 幂等加载挂载点清单（并发共享同一 promise；失败不置 loaded，下次可重试） */
export async function loadMounts(): Promise<void> {
  if (state === 'loaded') return
  if (!loadingPromise) {
    state = 'loading'
    loadingPromise = (async () => {
      try {
        mounts.value = await fetchMounts()
        state = 'loaded'
      } finally {
        loadingPromise = null
        if (state !== 'loaded') state = 'idle'
      }
    })()
  }
  await loadingPromise
}

/** 强制重拉挂载点清单（切换系统目录 / 收到变更事件后调用） */
export async function reloadMounts(): Promise<void> {
  mounts.value = await fetchMounts()
  state = 'loaded'
}

/**
 * 应用外壳侧边栏：`.vdfs` 虚拟根 → NavRail 项。
 *
 * 只取**导航可见**的挂载点：可见性由后端机制层声明（`VdfsProvider::nav_visible`），
 * 前端按标记过滤而不按挂载名硬编码排除——被隐藏的子树（如本地文件）依然可经
 * `.vdfs/<挂载名>` 寻址访问。
 *
 * 变更驱动：订阅 `vdfs` 事件总线，任一挂载点发生增删改即重拉清单
 * （挂载点集合变化极低频，整表重拉最简且正确）。
 */
export function useNavRailItems() {
  const route = useRoute()

  const navItems = computed<NavRailItem[]>(() =>
    mounts.value.filter(mountNavVisible).map((m) => ({
      key: m.mount,
      label: m.label || m.mount,
      icon: mountIconOf(m.mount) ?? undefined,
      description: m.description,
      active: route.path === navTargetOf(m.mount),
    }))
  )

  const unsubscribe = subscribe({ kind: VFDS_EVENT_KIND }, () => {
    void reloadMounts()
  })
  onBeforeUnmount(unsubscribe)

  function onNavSelect(mount: string): string {
    return navTargetOf(mount)
  }

  return { navItems, navTargetOf, onNavSelect }
}
