<!--
  VdfsView — VDFS 地址页（路由宿主，全 App 唯一的资源页宿主）

  本文件只做**两个地址概念之间的映射与装配**，三栏渲染与数据全部在三栏
  控件 `VdfsWorkbench`（唯一实现）里：

  - **数据地址 → 浏览器地址**：`/vdfs` ↔ `<根>`（首页）；
    `/vdfs/<dir…>` ↔ `<根>/<dir…>`（push 出来的地址页，如会话内部
    `<根>/session/<id>`）。两者是不同概念，本页只负责换算，数据加载
    由控件按绑定地址自取；
  - **首页**（绑 `<根>`）：左下角注入系统目录入口；
  - **有 push 来源的子地址页**（用户钻进的页面，地址深过一级类别）：左上角
    注入返回键——**回到 push 来源页**（浏览器历史 back），不是父目录。冷启动
    落点（会话目录，守卫 `replace` 进去、无来源）、深链直开、以及**左栏切类别**
    得到的一级页（`/vdfs/<挂载名>`）都不显示——那都是平级落点，不是「进入子页」。

  控件对绑定的是 `<根>` 还是 `<根>/session/<id>` 毫无感知：左栏 = 绑定地址
  的子目录导航，点左栏项中栏切换内容，点列表项右栏出详情——首页与内部
  管理页因此是**同一个控件、同一份数据逻辑**（useVdfs），只有绑定地址不同。

  本页是全 App **唯一**的资源页：VDFS 按**路径与访问位**组织资源，
  导航来自 `<根>` 的子目录清单，能力来自 r/w/l/t。新增一类资源只需后端
  实现一个 VdfsProvider 并挂载（`<根>/<插件名>/…` 是唯一地址），前端零开发。
  规范：docs/003-design/009-VDFS规范.md。

  ## S11 UI 约定

  - **列表顶部不出现面包屑**：路径即导航，层级靠左栏切目录 + 中栏点目录钻入 +
    左上角返回键逐级上溯；
  - **列表头不出现「新建目录」按钮**：新建 = 新建一种类型（由节点
    `new_type` 声明，至多一个），多于一条入口时先选入口再命名（在右栏详情区完成）；
  - **「浏览内部」/ open-container = push 页面**：进入节点同名目录地址，
    由同一个 VdfsView 承接；返回键逐级回到出发点。
-->
<template>
  <VdfsWorkbench
    ref="workbenchRef"
    :key="reloadKey"
    :addr="addr"
    :select="focus"
    @open="onOpen"
  >
    <template #rail-header>
      <!--
        页面**最左上角**这一格在首页/一级页是命令面板入口，深页面则是返回键。
        不放品牌 logo（主窗口菜单栏已有一枚，侧栏再来一枚是同屏重复）。

        返回键做成**整行的导航件**（占满侧栏宽 + 下方一条分隔线），而不是挤在
        56px 里的描边小胶囊——后者既塞不下「返回」二字，又和一排 40px 图标
        按钮互相打架。分隔线把它与下面的类别列表分开，读作窗口级的导航件。
      -->
      <button v-if="showBack" class="rail-back" title="返回" aria-label="返回" @click="goBack">
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="15 18 9 12 15 6" />
        </svg>
      </button>
      <button
        v-else
        class="rail-command"
        title="命令面板（Ctrl/Cmd+K）"
        aria-label="打开命令面板"
        aria-keyshortcuts="Control+k Meta+k"
        @click="emit('open-command-palette')"
      >
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="11" cy="11" r="6.5" />
          <path d="m16 16 4 4" />
        </svg>
      </button>
    </template>

    <!-- 左下角：系统目录切换（首页与地址页通用入口） -->
    <template #rail-footer>
      <HomedirEntry @reloaded="onHomedirReloaded" />
    </template>
  </VdfsWorkbench>
</template>

<script lang="ts">
/** 未知地址的一次性提示契约：路由只负责带标记，本页消费后清理。 */
export const UNKNOWN_ROUTE_QUERY = 'routeNotFound'
export const UNKNOWN_ROUTE_FLAG = '1'
export const UNKNOWN_ROUTE_MESSAGE = '这个地址不存在，已带你回到可用页面'
</script>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate, useRoute, useRouter } from 'vue-router'
import VdfsWorkbench from '@/components/vdfs/VdfsWorkbench.vue'
import HomedirEntry from '@/components/common/HomedirEntry.vue'
import { useSessionsStore } from '@/stores/sessions'
import { useToast } from '@/composables/useToast'
import { vdfsAddrOf, vdfsBrowserPathOf, shouldShowBack } from '@/schemas/vdfsAddress'

const route = useRoute()
const router = useRouter()
const workbenchRef = ref<InstanceType<typeof VdfsWorkbench> | null>(null)
const emit = defineEmits<{ (e: 'open-command-palette'): void }>()
const { showToast } = useToast()

onBeforeRouteLeave(() => workbenchRef.value?.confirmRouteLeave() ?? true)
// `/vdfs/:dir` 的目录切换复用同一路由组件，Back/Forward 触发的是 update 而非 leave。
onBeforeRouteUpdate((to) => {
  // 清理未知地址标记只改 query、不换页面，不能拿它去打断工作台里的未保存草稿。
  if (
    route.query[UNKNOWN_ROUTE_QUERY] === UNKNOWN_ROUTE_FLAG &&
    to.path === route.path &&
    to.query[UNKNOWN_ROUTE_QUERY] == null
  ) {
    return true
  }
  return workbenchRef.value?.confirmRouteLeave() ?? true
})

/**
 * 未知地址提示是一次性的：先展示，再原地 replace 掉标记。正常首页、打包入口与
 * 深链都没有标记；刷新时 URL 已清理，也不会再次提示。watch 同时覆盖初次落点
 * 与同一路由组件内的兜底跳转。
 */
watch(
  () => route.query[UNKNOWN_ROUTE_QUERY],
  (marker) => {
    if (marker !== UNKNOWN_ROUTE_FLAG) return
    showToast('info', UNKNOWN_ROUTE_MESSAGE)
    const query = { ...route.query }
    delete query[UNKNOWN_ROUTE_QUERY]
    void router.replace({
      path: route.path,
      query,
      ...(route.hash ? { hash: route.hash } : {}),
    })
  },
  { immediate: true },
)

// ==================== 数据地址 ↔ 浏览器地址 ====================
//
// 换算规则在 `schemas/vdfsAddress`（唯一来源）：路由宿主不该持有第二份，
// 且那里才测得动（本文件依赖 vue-router / store / 子组件）。

/** 绑定给控件的数据地址：`/vdfs` → `<根>`；`/vdfs/<dir…>` → `<根>/<dir…>` */
const addr = computed(() => vdfsAddrOf(route.params.dir as string | string[] | undefined))

/**
 * 到达后要顺带选中的**末段名**（`?select=`）。
 *
 * 文件没有自己的地址页（详情在三栏的右栏），所以「打开文件」的地址形态是
 * 「父目录 + `?select=<文件名>`」。只接受字符串；数组 / 缺省一律视为不选——
 * 形状不对时宁可落到目录页，也不猜。
 */
const focus = computed(() => {
  const s = route.query.select
  return typeof s === 'string' && s.trim() ? s : null
})

/**
 * 控件请求打开某地址：push 对应的浏览器地址（新地址页，同一个 VdfsView +
 * 同一个控件承接；返回键回 push 来源页）。
 *
 * 带 `select` 时（打开文件）也把选中项写进 URL——刷新/返回后仍是同一个视图；
 * 路径与选中项都相同则不 push（避免同一地点在历史里叠两条）。
 */
function onOpen(target: string, select?: string) {
  const path = vdfsBrowserPathOf(target)
  if (path === route.path && (select ?? null) === focus.value) return
  void router.push({ path, query: select ? { select } : {} })
}

// ==================== 返回键（有 push 来源的子页，左上角） ====================

/**
 * 返回键只在「真的从别处钻进子页」时显示（两个条件同时成立）。
 *
 * 冷启动落点是会话目录——地址形状上是深页，但那是守卫 `replace` 进去的，
 * 没有来源页可返，并不是用户「进入子页面」；此时左上角必须空着。
 * `history.state.back` 非空才是 push 进来的证据。
 *
 * 另一个条件是**地址深过一级类别页**：左栏切类别（会话 → 模型）走的是 push，
 * 天然有来源，但那是平级切换，不是钻入；`/vdfs/<挂载名>` 一律不显示返回键。
 * 判据本体在 `schemas/vdfsAddress.shouldShowBack`，这里只喂路由与历史状态。
 *
 * `history.state` 不是 Vue 响应式源，靠 `route.fullPath` 的变化触发重算：
 * push / back 改变地址时连带更新它，原地 replace 到同一地址则不变。
 */
const showBack = computed(() =>
  shouldShowBack(route.fullPath, window.history.state?.back != null),
)

/**
 * 返回 = 回 **push 来源页**（浏览器历史 back），不是父目录——push 之前在哪个
 * 页面就回哪里。`showBack` 已保证「无来源时不显示这枚键」，这里的 `else` 只是
 * 同一判据的防御性兜底（深链直开万一被外部触发时仍有路回首页）。
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
/*
 * 侧栏顶部两枚入口：一级页显示命令面板，深页面显示返回。两者共用整行导航件
 * 的视觉语言；命令入口只保留图标，返回键在深页面保持整行箭头。
 */
.rail-command {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.2rem;
  width: 100%;
  height: var(--workbench-bar-height);
  flex-shrink: 0;
  padding: 0;
  border: none;
  border-bottom: 1px solid var(--border-default);
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease;
}
.rail-command:hover {
  background: var(--surface-hover);
  color: var(--text-primary);
}
.rail-command svg {
  width: 1.125rem;
  height: 1.125rem;
}

.rail-back {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: var(--workbench-bar-height);
  flex-shrink: 0;
  padding: 0;
  border: none;
  border-bottom: 1px solid var(--border-default);
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease;
}

.rail-back:hover {
  background: var(--surface-hover);
  color: var(--text-primary);
}

.rail-back svg {
  width: 1.25rem;
  height: 1.25rem;
}
</style>
