<!--
  VdfsView — VDFS 地址页（路由宿主，全 App 唯一的资源页宿主）

  本文件只做**两个地址概念之间的映射与装配**，三栏渲染与数据全部在三栏
  控件 `VdfsWorkbench`（唯一实现）里：

  - **数据地址 → 浏览器地址**：`/vdfs` ↔ `.vdfs`（首页）；
    `/vdfs/<dir…>` ↔ `.vdfs/<dir…>`（push 出来的地址页，如会话内部
    `.vdfs/session/<id>`）。两者是不同概念，本页只负责换算，数据加载
    由控件按绑定地址自取；
  - **首页**（绑 `.vdfs`）：左下角注入系统目录入口；
  - **非首页**（push 出来的地址页）：左上角注入返回键——**回到 push 来源页**
    （浏览器历史 back；深链直开无来源时回首页），不是父目录。

  控件对绑定的是 `.vdfs` 还是 `.vdfs/session/<id>` 毫无感知：左栏 = 绑定地址
  的子目录导航，点左栏项中栏切换内容，点列表项右栏出详情——首页与内部
  管理页因此是**同一个控件、同一份数据逻辑**（useVdfs），只有绑定地址不同。

  本页是全 App **唯一**的资源页：VDFS 按**路径与访问位**组织资源，
  导航来自 `.vdfs` 的子目录清单，能力来自 r/w/l/t。新增一类资源只需后端
  实现一个 VdfsProvider 并挂载（`.vdfs/<插件名>/…` 是唯一地址），前端零开发。
  规范：docs/design/vdfs.md。

  ## S11 UI 约定

  - **列表顶部不出现面包屑**：路径即导航，层级靠左栏切目录 + 中栏点目录钻入 +
    左上角返回键逐级上溯；
  - **列表头不出现「新建目录」按钮**：新建 = 新建一种类型（清单由节点
    `new_types` 声明），多类型时先选类型再命名（在右栏详情区完成）；
  - **「浏览内部」/ open-container = push 页面**：进入节点同名目录地址，
    由同一个 VdfsView 承接；返回键逐级回到出发点。
-->
<template>
  <VdfsWorkbench :key="reloadKey" :addr="addr" @open="onOpen">
    <template #rail-header>
      <!--
        push 出来的地址页：左上角是**返回按钮**（带文字，不是光秃秃一个图标），
        且**不显示主 logo**——它是首页的品牌位，跟着 push 进每一层子页面
        既无意义，也正好占掉返回键该在的位置。
      -->
      <button v-if="showBack" class="rail-back" title="返回" aria-label="返回" @click="goBack">
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="19" y1="12" x2="5" y2="12" />
          <polyline points="12 19 5 12 12 5" />
        </svg>
        <span class="rail-back-text">返回</span>
      </button>
      <!-- 首页：主 logo（正式品牌资源 `assets/logo.svg`，不再是占位字母块） -->
      <div v-else class="logo-area">
        <img :src="logoUrl" alt="Symbio" class="app-logo" />
      </div>
    </template>

    <!-- 左下角：系统目录切换（首页与地址页通用入口） -->
    <template #rail-footer>
      <HomedirEntry @reloaded="onHomedirReloaded" />
    </template>
  </VdfsWorkbench>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import VdfsWorkbench from '@/components/vdfs/VdfsWorkbench.vue'
import HomedirEntry from '@/components/common/HomedirEntry.vue'
import { useSessionsStore } from '@/stores/sessions'
import { VDFS_ROOT } from '@/schemas/vdfs'
// 正式品牌资源（与「关于」页同源；此前主窗口用的是一个占位字母块）
import logoUrl from '../assets/logo.svg'

const route = useRoute()
const router = useRouter()

// ==================== 数据地址 ↔ 浏览器地址 ====================

/** 绑定给控件的数据地址：`/vdfs` → `.vdfs`；`/vdfs/<dir…>` → `.vdfs/<dir…>` */
const addr = computed(() => {
  const d = route.params.dir
  const rel = (Array.isArray(d) ? d.join('/') : ((d as string) || '')) as string
  return rel ? `${VDFS_ROOT}/${rel}` : VDFS_ROOT
})

/** 数据地址 → 浏览器地址（`.vdfs/<rel>` → `/vdfs/<rel>`；根 → `/vdfs`） */
function browserPathOf(target: string): string {
  if (target === VDFS_ROOT) return '/vdfs'
  if (target.startsWith(`${VDFS_ROOT}/`)) return `/vdfs/${target.slice(VDFS_ROOT.length + 1)}`
  return '/vdfs'
}

/**
 * 控件请求钻入某目录的数据地址：push 对应的浏览器地址（新地址页，
 * 同一个 VdfsView + 同一个控件承接；返回键回 push 来源页）。
 */
function onOpen(target: string) {
  const path = browserPathOf(target)
  if (path !== route.path) void router.push(path)
}

// ==================== 返回键（非首页地址页，左上角） ====================

/** 首页 = `/vdfs`（数据地址 `.vdfs`）；更深的地址都是 push 出来的页面 */
const showBack = computed(() => route.path.startsWith('/vdfs/'))

/**
 * 返回 = 回 **push 来源页**（浏览器历史 back），不是父目录——push 之前在哪个
 * 页面就回哪里。深链直开（无 push 来源）时回首页兜底。
 */
function goBack() {
  if (window.history.state?.back != null) router.back()
  else void router.push('/vdfs')
}

// ==================== 系统目录切换（左下角入口的联动） ====================

/** 切换系统目录后重挂控件（连接目标已变，数据整体重来） */
const reloadKey = ref(0)

async function onHomedirReloaded() {
  // 会话清单来自远端实例，切换后必须重拉
  void useSessionsStore().refreshList()
  // 回首页（避免停留在某个「已失效」的地址页）
  if (route.path !== '/vdfs' && route.path !== '/') void router.push('/vdfs')
  reloadKey.value++
}
</script>

<style scoped>
/* 首页品牌位：正式 logo 资源（assets/logo.svg，512×512 带圆角底板） */
.logo-area {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0.75rem 0;
  border-bottom: 1px solid var(--border-default);
}

.app-logo {
  width: 2rem;
  height: 2rem;
  display: block;
  flex-shrink: 0;
  border-radius: var(--radius-md);
}

/*
 * push 页的返回按钮：**带文字的按钮**，不是光秃秃一个图标。
 *
 * 侧栏只有 56px（`--sidebar-width`），故图标与文字紧凑横排、字号压到 0.7rem；
 * 加一条边框让它读作「按钮」而不是又一个导航项——这正是原实现（一个裸箭头
 * 图标）被读成「图标」的原因。
 */
.rail-back {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.25rem;
  width: calc(100% - 0.75rem);
  margin: 0.5rem auto 0;
  padding: 0.4rem 0;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-secondary);
  font-size: 0.7rem;
  font-family: inherit;
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease;
}

.rail-back:hover {
  background: var(--surface-hover);
  color: var(--text-primary);
}

.rail-back-text {
  white-space: nowrap;
  line-height: 1;
}
</style>
