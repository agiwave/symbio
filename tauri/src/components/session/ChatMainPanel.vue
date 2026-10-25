<template>
  <div class="chat-main-panel">
    <header v-if="hasActive" class="chat-header">
      <div class="header-left">
        <!-- 内联重命名：原地把标题变成输入框。不用浏览器原生 prompt——
             它阻塞主线程、样式与主题脱节、且无法做焦点管理。 -->
        <input
          v-if="renaming"
          ref="renameInputRef"
          v-model="draftTitle"
          class="rename-input"
          spellcheck="false"
          @keyup.enter="submitRename"
          @keyup.esc="cancelRename"
          @blur="cancelRename"
        />
        <template v-else>
          <h2 class="session-name">{{ store.activeTitle }}</h2>
          <span v-if="store.isActiveWorking" class="status-working">● AI 处理中</span>
        </template>
        <!-- 会话头的用量读数：**一格**，由后端在交付出口排好（`session/stats.usage`）。
             这里不写任何标签 / 单位 / 换算——`ReadingMeter` 与抽屉里的面板是同一个
             方言的两个渲染器，改口径只改后端。缺数（一次实测都没有）整格不出现。 -->
        <ReadingMeter class="header-meter" :section="stats?.usage" />
      </div>
      <div class="header-right">
        <button class="header-btn" title="重命名" @click="startRename">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M12 20h9" />
            <path d="M16.5 3.5a2.121 2.121 0 1 1 3 3L7 19l-4 1 1-4 12.5-12.5z" />
          </svg>
        </button>
        <!-- 变更抽屉开关：本会话改过哪些文件（只读投影的出口）。点行打开的是
             **当前**文件——经既有 VDFS 树，不新建专属页面、不画 diff。 -->
        <button
          class="header-btn"
          :class="{ 'is-active': changesOpen }"
          title="变更"
          :disabled="!hasActive"
          @click="changesOpen = !changesOpen"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
            <polyline points="14 2 14 8 20 8" />
            <line x1="8" y1="13" x2="16" y2="13" />
            <line x1="8" y1="17" x2="13" y2="17" />
          </svg>
        </button>
        <!-- 读数抽屉开关（P4）：按钮只声明「打开」，面板里显示什么由后端下发的
             描述决定——这里不出现任何读数语义。 -->
        <button
          class="header-btn"
          :class="{ 'is-active': panelOpen }"
          title="读数"
          :disabled="!hasActive"
          @click="panelOpen = !panelOpen"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M4 19V10M10 19V5M16 19v-7M22 19H2" />
          </svg>
        </button>
        <!-- 动作区 = 会话自有动作（进入下一级）+ 机制动作（删除），
             去重合并由机制唯一实现（schemas/vdfs-form.mergeDetailActions）。
             **排在最后**：写在本区末位，机制再把「进入下一级」归到行尾
             （`mergeDetailActions`），于是它落在整个头部的最右端——
             它是本页唯一的**导航**出口，与前面「作用于本页」的动作（重命名 /
             删除）语义不同，用左边框分隔开。 -->
        <VdfsActions
          v-if="merged.actions.length"
          class="header-actions"
          :actions="merged.actions"
          :busy="merged.busy"
          @run="(a) => emit('action', a)"
        />
      </div>
    </header>

    <main class="chat-body">
      <template v-if="!hasActive">
        <div class="no-session">
          <p class="no-session-title">未选择会话</p>
          <p class="no-session-desc">从左侧选择或创建一个会话开始</p>
        </div>
      </template>

      <template v-else-if="loadError">
        <div class="load-error">
          <p class="load-error-icon">⚠</p>
          <p class="load-error-title">加载会话历史失败</p>
          <p class="load-error-desc">{{ loadError }}</p>
          <div class="load-error-actions">
            <button class="load-error-btn primary" @click="reloadCurrent">重试</button>
            <button class="load-error-btn" @click="dismissError">忽略</button>
          </div>
        </div>
      </template>

      <template v-else-if="!messagesReady || currentLoadedId !== store.activeId">
        <div class="chat-loading">
          <p>正在加载会话历史…</p>
        </div>
      </template>

      <!-- 工作目录判断：放在"会话详情已加载"之后（编辑器获得当前会话详情后再判定）。
           会话自身是否有 workdir 以详情/metadata 为准，不再仅凭列表卡片即时弹引导。 -->
      <template v-else-if="!store.activeWorkdir">
        <EmptyWorkdirState />
      </template>

      <template v-else>
        <ModelChatPanel
          :key="store.activeAddr || 'none'"
          :sessionId="store.activeId ?? ''"
          :sessionAddr="store.activeAddr"
        />
      </template>
    </main>

    <!-- 读数抽屉（P4）。**滑出式**而不是占版面：读数是要「按一下看一眼」的东西，
         常驻会把对话区挤窄，而对话才是这个面板的主体。
         内容由后端下发的描述渲染（ReadingPanel）——本文件不写任何读数语义。 -->
    <aside v-if="panelOpen && hasActive" class="reading-drawer">
      <header class="drawer-head">
        <span class="drawer-title">读数</span>
        <div class="drawer-actions">
          <button class="drawer-btn" :disabled="statsLoading" title="重新读取" @click="loadStats">
            {{ statsLoading ? '读取中…' : '刷新' }}
          </button>
          <button class="drawer-btn" title="关闭" @click="panelOpen = false">✕</button>
        </div>
      </header>

      <div class="drawer-body">
        <p v-if="statsError" class="drawer-error">{{ statsError }}</p>
        <p v-else-if="statsLoading && !stats" class="drawer-loading">正在读取读数…</p>
        <!-- 空态：没有事实源 ⇒ 后端仍给一份合法面板（sections 为空或只有提示行）。
             「没有数」与「读不到」由 panel 自己的行区分，这里不再判一次。 -->
        <ReadingPanel v-else-if="stats" :panel="stats.panel" />
      </div>
    </aside>

    <!-- 变更抽屉：与读数抽屉同一个容器形态、同一时刻只开一个（`drawer` 是唯一状态）。
         行由后端排好（路径 / 动作词 / 轮号 / 次数），本文件只做开关、代次守卫与
         错误上浮；点行把「会话地址 + 后端给的相对地址」拼成目标地址交给页面层
         （渲染器不导航，见 VdfsWorkbench 的 open-node 约定）。 -->
    <aside v-if="changesOpen && hasActive" class="reading-drawer">
      <header class="drawer-head">
        <span class="drawer-title">变更</span>
        <div class="drawer-actions">
          <button class="drawer-btn" :disabled="changesLoading" title="重新读取" @click="loadChanges">
            {{ changesLoading ? '读取中…' : '刷新' }}
          </button>
          <button class="drawer-btn" title="关闭" @click="changesOpen = false">✕</button>
        </div>
      </header>

      <div class="drawer-body">
        <p v-if="changesError" class="drawer-error">{{ changesError }}</p>
        <p v-else-if="changesLoading && !changes" class="drawer-loading">正在读取变更…</p>
        <SessionChangesList v-else-if="changes" :changes="changes" @open="openChange" />
      </div>
    </aside>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { useSessionsStore } from '@/stores/sessions'
import { logger } from '@/utils/logger'
import EmptyWorkdirState from './EmptyWorkdirState.vue'
import ModelChatPanel from '../ModelChatPanel.vue'
import VdfsActions from '@/components/vdfs/VdfsActions.vue'
import ReadingPanel from '@/components/common/ReadingPanel.vue'
import ReadingMeter from '@/components/common/ReadingMeter.vue'
import SessionChangesList from './SessionChangesList.vue'
import { useGenerationGuard } from '@/composables/useGenerationGuard'
import { mergeDetailActions, vdfsJoin, type DetailAction } from '@/schemas/vdfs'
import { readSessionStats, type SessionStats } from '@/services/sessionStats'
import {
  readSessionChanges,
  type ChangeRow,
  type SessionChanges,
} from '@/services/sessionChanges'

const props = defineProps<{
  /** 会话自有动作（如「进入下一级」），由 Session ← VdfsSessionDetail 从节点定义投影 */
  actions?: DetailAction[]
  /** 与 `actions` 等长且按索引对齐的禁用标记（定义 `disabled_when` 的投影结果） */
  actionDisabled?: boolean[]
  /** 机制动作注入（页面单一定义点计算：删除等），与自身按钮并排渲染 */
  mechanismActions?: DetailAction[]
  /** 正在执行的机制动作 id（驱动其进行中文案） */
  mechanismBusy?: string | null
}>()

const emit = defineEmits<{
  /** 动作上抛（Session 分发到页面层机制通道：delete / open-container） */
  (e: 'action', action: DetailAction): void
  /** 打开一个 VDFS 节点（变更行指向的文件）；呈现方式由页面层决定 */
  (e: 'open-node', addr: string): void
}>()

/** 动作区 = 自有 + 机制（去重合并、位置与忙态/禁用对齐由机制唯一实现） */
const merged = computed(() =>
  mergeDetailActions(
    props.actions ?? [],
    props.mechanismActions ?? [],
    [],
    props.actionDisabled ?? [],
    props.mechanismBusy ?? null
  )
)

const store = useSessionsStore()

const hasActive = computed(() => !!store.activeId)

// 跟踪当前活跃会话的消息是否已经从后端拉取过。
// ModelChatPanel 只会在拿到"权威"历史后才挂载，避免：
// 1) UI 先用本地缓存的中间态（streaming/waiting）渲染，
// 2) 紧接着被后端的最终态（completed/failed）覆盖的闪烁。
const messagesReady = ref(false)
const currentLoadedId = ref<string | null>(null)
const loadError = ref<string | null>(null)

/**
 * 会话切换的**代次守卫**（机制唯一实现，见 `useGenerationGuard`）。
 *
 * 快速连切会话时，先发出的 `loadMessages` 可能后回来——它会用旧会话的消息
 * 覆盖新会话的界面。取号 → 响应回来比对 → 过期即丢。
 */
const loadGuard = useGenerationGuard()

watch(
  () => store.activeId,
  async (id) => {
    const rev = loadGuard.advance()
    messagesReady.value = false
    currentLoadedId.value = null
    loadError.value = null
    if (id) {
      // 打开会话 ⇒ 补列一次收件箱（排队中的消息）。补的是**冷启动窗口**：应用起来
      // 之前就排在队里的消息没有入队帧可收（见 `stores/sessionInboxSync` 模块头的
      // 「已知代价」）。不 await、也不过守卫：它是可选的可视化，失败只少几个气泡，
      // 转写照旧加载（store 内部已把失败吞成 warn）。
      void store.refreshInbox(id)
      try {
        await store.loadMessages(id)
        // 检查是否被新的切换打断
        if (loadGuard.revision() !== rev) {
          logger.debug('ChatMainPanel', `loadMessages(${id}) was superseded by a later switch`)
          return
        }
        currentLoadedId.value = id
        messagesReady.value = true
      } catch (e) {
        if (loadGuard.revision() !== rev) return
        const msg = e instanceof Error ? e.message : String(e || '未知错误')
        loadError.value = msg
        // 仍然把 currentLoadedId 设为 id，避免 messagesReady 永久 false
        // 让用户可以"忽略"继续发送消息
        currentLoadedId.value = id
        messagesReady.value = true
        logger.error('ChatMainPanel', `loadMessages(${id}) failed`, e)
      }
    }
  },
  { immediate: true }
)

/**
 * 重试当前会话的历史加载。
 *
 * 同样过守卫：重试期间用户切走了会话，这次响应不得落地——原先直接读
 * `store.activeId`，会把**新会话**标成已加载，而装的其实是旧会话的消息。
 */
async function reloadCurrent() {
  const id = store.activeId
  if (!id) return
  const rev = loadGuard.advance()
  loadError.value = null
  messagesReady.value = false
  try {
    await store.loadMessages(id)
    if (loadGuard.revision() !== rev) return
    currentLoadedId.value = id
    messagesReady.value = true
  } catch (e) {
    if (loadGuard.revision() !== rev) return
    const msg = e instanceof Error ? e.message : String(e || '未知错误')
    loadError.value = msg
    messagesReady.value = true
  }
}

function dismissError() {
  loadError.value = null
}

/** 内联重命名（原地编辑标题；不用浏览器原生 prompt） */
const renaming = ref(false)
const draftTitle = ref('')
const renameInputRef = ref<HTMLInputElement | null>(null)

async function startRename() {
  if (!store.activeId) return
  draftTitle.value = store.activeTitle
  renaming.value = true
  await nextTick()
  renameInputRef.value?.select()
}

function cancelRename() {
  renaming.value = false
}

/** 提交重命名（空标题或与原名相同视为放弃——不做无意义的往返） */
async function submitRename() {
  const id = store.activeId
  if (!id || !renaming.value) return
  const title = draftTitle.value.trim()
  renaming.value = false
  if (!title || title === store.activeTitle) return
  await store.rename(id, title)
}

// ── 读数抽屉（P4）────────────────────────────────────────────────────────
//
// ## 这里不出现任何读数语义
//
// 按钮只声明「打开 / 关闭」，`loadStats` 只负责把路由结果搬进 `stats`，
// 面板里显示什么由后端下发的 `panel` 描述决定（`ReadingPanel` 组件排版）。
// 本文件的全部逻辑只有三件：开关、代次守卫、错误上浮。
//
// ## 为什么过代次守卫
//
// 读数是一次**往返**。抽屉开着时连切会话，先发的那次可能后回来——它会把旧会话的
// 读数显示在新会话的抽屉里，而两个文件看起来都正常（数字不报错）。
// 与消息加载同一个守卫机制（`useGenerationGuard`），不另写一套。

// 两个抽屉共用右侧同一块位置，因此「开着的是哪一个」是**一份状态**而不是两个
// 布尔——两个布尔会出现两个抽屉同时滑出、互相盖住，而界面看起来仍然"正常"。
type PanelDrawer = 'reading' | 'changes'
const drawer = ref<PanelDrawer | null>(null)
const panelOpen = computed({
  get: () => drawer.value === 'reading',
  set: (open: boolean) => {
    drawer.value = open ? 'reading' : null
  },
})
const changesOpen = computed({
  get: () => drawer.value === 'changes',
  set: (open: boolean) => {
    drawer.value = open ? 'changes' : null
  },
})
const stats = ref<SessionStats | null>(null)
const statsLoading = ref(false)
const statsError = ref<string | null>(null)

// 变更清单与读数并列（同一份面板的两个抽屉）。声明在这一段而不是文件末尾：会话切换
// 的 watch 是 `immediate`，在 setup 期间就会写它们——晚于它声明会落进暂时性死区。
const changes = ref<SessionChanges | null>(null)
const changesLoading = ref(false)
const changesError = ref<string | null>(null)

/**
 * 读当前会话的读数。
 *
 * 失败**不静默**：抽屉里显示错误行而不是空白——空白会被读成「这个会话没有读数」，
 * 而事实可能是路由不可用或权限不足。两者的处置完全不同。
 */
async function loadStats() {
  const id = store.activeId
  if (!id) return
  const rev = panelGuard.advance()
  statsLoading.value = true
  statsError.value = null
  try {
    const got = await readSessionStats(id)
    // 代次过期即丢：抽屉已经切到别的会话，这次响应不属于当前界面
    if (panelGuard.revision() !== rev) return
    stats.value = got
  } catch (e) {
    if (panelGuard.revision() !== rev) return
    statsError.value = e instanceof Error ? e.message : String(e || '读取读数失败')
    logger.error('ChatMainPanel', `readSessionStats(${id}) failed`, e)
  } finally {
    if (panelGuard.revision() === rev) statsLoading.value = false
  }
}

/** 抽屉自己的代次守卫（与消息加载分开：两条往返互不打断计数）。 */
const panelGuard = useGenerationGuard()

// 会话头的用量格也要这份读数，于是它从「抽屉的东西」变成「这个面板的东西」：
// 切会话时读一次（下面那个 watch），一轮跑完再读一次。
//
// **仍然不轮询**：读一次要一整套投影（`fleet` 列是部署级扫描，见 `stats.rs`），
// 按时间轮询等于把成本乘上秒数。这里只在**事实变了**的两个时刻读——换了会话、
// 这一轮从跑到不跑。中间用户没做任何事的时候，界面上的数不该变。
watch(panelOpen, (open) => {
  // 打开抽屉时若已经有这一份就不再读：会话头已经拉过，重复读只是白花代价。
  if (open && !stats.value) void loadStats()
})

// 切会话 ⇒ 抽屉内容与头上的读数一起作废，并立刻为**新会话**读一次（头上的读数
// 是常驻的，不能等用户点开抽屉才出现）。代次守卫负责把先发后到的旧响应丢掉。
// `immediate`：从路由恢复会话时 `activeId` 在挂载前就有值，等不到「变化」。
watch(
  () => store.activeId,
  () => {
    stats.value = null
    statsError.value = null
    void loadStats()
    // 变更清单同理：旧会话的行不能留在新会话的抽屉里。面板开着才读——关着时
    // 没有消费方，提前读只是白花一次投影。
    changes.value = null
    changesError.value = null
    if (changesOpen.value) void loadChanges()
  },
  { immediate: true }
)

// 一轮跑完（working → 不 working）⇒ 重读一次：这一轮新落的 `response` 里有实测用量，
// 头上的累计值正是「跑完一轮」才该变的东西。变更清单同理——这一轮新落的
// `artifact.added` 正是它要回答的东西。
watch(
  () => store.isActiveWorking,
  (now, was) => {
    if (!was || now) return
    void loadStats()
    if (changesOpen.value) void loadChanges()
  }
)

// ── 变更抽屉 ────────────────────────────────────────────────────────────
//
// 行怎么排、哪些工具算写、空态是哪一种，全部由后端定（`SessionChangesList`
// 只是排版器）。本文件负责：开关、代次守卫、错误上浮，以及把「相对地址」交给
// 页面层——渲染器不导航（点行上抛 `open-node`，由三栏控件决定 push）。

/** 变更清单自己的代次守卫（与读数、消息加载各不相干，互不打断计数）。 */
const changesGuard = useGenerationGuard()

watch(changesOpen, (open) => {
  // 打开时若已经有这一份就不再读：刷新按钮与轮末重读负责更新。
  if (open && !changes.value) void loadChanges()
})

/** 读当前会话的变更清单；失败不静默（空表会被读成「没改过」）。 */
async function loadChanges() {
  const id = store.activeId
  if (!id) return
  const rev = changesGuard.advance()
  changesLoading.value = true
  changesError.value = null
  try {
    const got = await readSessionChanges(id)
    if (changesGuard.revision() !== rev) return
    changes.value = got
  } catch (e) {
    if (changesGuard.revision() !== rev) return
    changesError.value = e instanceof Error ? e.message : String(e || '读取变更失败')
    logger.error('ChatMainPanel', `readSessionChanges(${id}) failed`, e)
  } finally {
    if (changesGuard.revision() === rev) changesLoading.value = false
  }
}

/**
 * 打开一行指向的文件：会话地址 + 后端给的相对地址 = 完整数据地址。
 *
 * 拼合走地址代数（`vdfsJoin`），本文件不认识 `workdir` 这个段名——它由后端随行
 * 下发。目标是一个**文件**；怎么把文件打开成页面（父目录 + 选中项）由页面层决定。
 */
function openChange(row: ChangeRow) {
  if (!store.activeAddr) return
  emit('open-node', vdfsJoin(store.activeAddr, row.address))
}
</script>

<style scoped>
.chat-main-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
  background: var(--surface-panel);
  overflow: hidden;
}

.chat-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 1rem;
  border-bottom: 1px solid var(--border-default);
  flex-shrink: 0;
  min-height: 2.75rem;
}

.header-left {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  min-width: 0;
}

.session-name {
  font-size: 0.95rem;
  font-weight: 500;
  color: var(--text-primary);
  margin: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 16.25rem;
}

.status-working {
  font-size: 0.75rem;
  color: #22c55e;
}

/* 会话头的用量格：占位随内容收缩，标题被压缩时它是**最后**让位的
   （标题是导航信息，用量是这一眼的读数——`flex-shrink: 0` 让它不被挤没）。 */
.header-meter {
  flex-shrink: 0;
  /* `ReadingMeter` 的默认内边距是给输入区那条线用的（底部留白），头部居中排，
     两端都不该多出空白。 */
  padding: 0;
}

/* 内联重命名的输入框（替换标题位；宽度与 .session-name 一致，避免布局跳动） */
.rename-input {
  font-family: inherit;
  font-size: 0.95rem;
  font-weight: 500;
  color: var(--text-primary);
  background: var(--surface-sunken);
  border: 1px solid var(--accent);
  border-radius: 0.375rem;
  padding: 0.15rem 0.5rem;
  width: 16.25rem;
  max-width: 100%;
  outline: none;
}

.header-right {
  display: flex;
  align-items: center;
  gap: 0.25rem;
}

/* 动作区（进入下一级 / 删除）与前面「作用于本页」的小图标按钮之间画一道分隔：
   它们是本页唯一的**导航**出口，语义与重命名不同，不该读成同一排工具。
   `align-self: stretch` 让分隔线跟着头部高度走，而不是跟着按钮走。 */
.header-actions {
  margin-left: 0.5rem;
  padding-left: 0.5rem;
  border-left: 1px solid var(--border-default);
}

.header-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 1.75rem;
  height: 1.75rem;
  border: none;
  background: transparent;
  border-radius: 0.375rem;
  cursor: pointer;
  color: var(--text-secondary);
  transition: all 0.15s;
}

.header-btn:hover {
  background: var(--surface-hover);
  color: var(--text-primary);
}

/* ── 读数抽屉（P4）───────────────────────────────────────────────────────
   绝对定位于右、宽度固定、带左边界：它盖在对话区之上而不是把它挤窄——
   对话是这个面板的主体，读数是「按一下看一眼」的旁支。 */
.reading-drawer {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  width: min(360px, 42vw);
  display: flex;
  flex-direction: column;
  background: var(--surface-panel);
  border-left: 1px solid var(--border-default);
  box-shadow: -8px 0 24px rgba(0, 0, 0, 0.12);
  z-index: 5;
}

.drawer-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 0.5rem 0.75rem;
  border-bottom: 1px solid var(--border-default);
  flex-shrink: 0;
}

.drawer-title {
  font-size: 0.8rem;
  font-weight: 600;
}

.drawer-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}

.drawer-btn {
  border: 1px solid var(--border-default);
  background: transparent;
  color: var(--text-secondary);
  border-radius: 4px;
  font-size: 0.7rem;
  padding: 2px 8px;
  cursor: pointer;
}

.drawer-btn:hover:not(:disabled) {
  color: var(--text-primary);
}

/* 抽屉开着 ⇒ 按钮呈按下态。颜色用 text-primary（与 hover 同一支），
   靠底色区分「开着」与「悬停」——不再引入新令牌。 */
.header-btn.is-active {
  color: var(--text-primary);
  background: var(--surface-hover, rgba(128, 128, 128, 0.12));
}

.drawer-btn:disabled {
  opacity: 0.5;
  cursor: default;
}

.drawer-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 0.75rem;
}

.drawer-loading,
.drawer-error {
  font-size: 0.75rem;
  color: var(--text-muted);
  margin: 0;
}

.drawer-error {
  color: var(--danger-fg);
}
.chat-body {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  display: flex;
}

.chat-body > * {
  flex: 1;
  min-height: 0;
}

.no-session {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  width: 100%;
  color: var(--text-muted);
  text-align: center;
  padding: 2rem;
}

.no-session-title {
  font-size: 1rem;
  margin-bottom: 0.4rem;
}

.no-session-desc {
  font-size: 0.85rem;
  opacity: 0.7;
}

.chat-loading {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--text-muted);
  font-size: 0.9rem;
}

/* ═══════════════════════════════════════════════════════════
   Load error state
   ═══════════════════════════════════════════════════════════ */
.load-error {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  text-align: center;
  color: var(--text-muted);
  padding: 2rem;
  gap: 0.5rem;
}

.load-error-icon {
  font-size: 2.5rem;
  color: var(--color-banner-border);
  margin-bottom: 0.5rem;
}

.load-error-title {
  font-size: 1rem;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0;
}

.load-error-desc {
  font-size: 0.85rem;
  color: var(--text-muted);
  margin: 0;
  max-width: 30rem;
  word-break: break-word;
  font-family: 'Fira Code', 'Consolas', monospace;
  background: var(--color-msg-card);
  padding: 0.4rem 0.8rem;
  border-radius: 0.375rem;
}

.load-error-actions {
  display: flex;
  gap: 0.5rem;
  margin-top: 1rem;
}

.load-error-btn {
  padding: 0.4rem 1rem;
  border-radius: 0.375rem;
  font-size: 0.85rem;
  border: 1px solid var(--border-default);
  background: var(--surface-panel);
  color: var(--text-primary);
  cursor: pointer;
  transition: all 0.15s ease;
}

.load-error-btn:hover {
  background: var(--surface-sunken);
}

.load-error-btn.primary {
  background: var(--accent);
  color: var(--text-on-accent);
  border-color: var(--accent);
}

.load-error-btn.primary:hover {
  opacity: 0.9;
}
</style>
