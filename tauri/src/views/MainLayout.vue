<!--
  MainLayout — 应用根布局（全局单例职责，不含任何页面/导航知识）

  三栏工作台由各路由页面自持（VdfsView 嵌入唯一的 VdfsWorkbench 控件），
  本文件不再有外壳三栏；只承担**跨页面全局**职责：

  - 路由出口（RouterView）；
  - 全局浮动消息浮层（Toast，状态来自 useToast 单例，全 App 仅渲染一次）；
  - 全局命令面板（Cmd/Ctrl+K，显隐只有本文件持有；键盘是全 App 的，面板也是）；
  - 全局初始化：VDFS 转写同步 + 工作区恢复（须先于页面挂载）。
-->
<template>
  <div class="main-layout">
    <!-- 不加 :key：路由变化由 VdfsView 依 route.params 换算数据地址，
         控件（VdfsWorkbench）按绑定的数据地址自行重载 -->
    <RouterView />
    <!-- 全局浮动消息浮层 -->
    <Toast />
    <!-- 首启引导（全 App 仅渲染一次；可见性由自己的 store 决定，
         且只在「该做的事还没做」时自动弹——见 stores/onboarding） -->
    <Onboarding />
    <!-- 全局命令面板（Cmd/Ctrl+K）：显隐只由下面这个全局键决定，不放进任何 store
         ——面板是本壳的职责（本文件是唯一持有它的人），SECOND source of truth 无益 -->
    <CommandPalette :visible="paletteOpen" @close="paletteOpen = false" />
  </div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { RouterView } from 'vue-router'
import { startSessionTranscriptSync, registerTranscriptMount } from '@/stores/sessionTranscriptSync'
import { startSessionNodeSync, registerSessionMount } from '@/stores/sessionNodeSync'
import { startSessionInboxSync, registerInboxMount } from '@/stores/sessionInboxSync'
import { setChimeSettingsSource } from '@/services/completionChime'
import { getWorkspacePath } from '@/services/home'
import { useSessionsStore } from '@/stores/sessions'
import { useSoundSettingsStore } from '@/stores/soundSettings'
import { useOnboardingStore } from '@/stores/onboarding'
import { logger } from '@/utils/logger'
import Toast from '@/components/common/Toast.vue'
import Onboarding from '@/components/common/Onboarding.vue'
import CommandPalette from '@/components/common/CommandPalette.vue'

const sessions = useSessionsStore()

/**
 * 命令面板的显隐（全 App 唯一一份）。
 *
 * 键盘口径沿用仓内既有那一条（`CodeEditor` 的 Ctrl+S）：`(ctrlKey || metaKey)`——
 * mac 上是 ⌘K、Windows/Linux 上是 Ctrl+K，一条判断覆盖两个平台。
 */
const paletteOpen = ref(false)

function onGlobalKeydown(e: KeyboardEvent) {
  if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return
  // 按**字符**而不是物理键位（`e.code`）判：非美式布局用户的肌肉记忆跟着字母走
  if (e.key.toLowerCase() !== 'k') return
  // 拦下浏览器默认（Chrome 的 Ctrl+K = 聚焦地址栏搜索）
  e.preventDefault()
  paletteOpen.value = !paletteOpen.value
}

/**
 * 当前空间的会话挂载目录一变，就向三个实时消费端各登记一次。
 *
 * **为什么必须在这里**：会话挂载目录不止一份——子智能体空间
 * （`<根>/agent/<id>/…`）是一棵完整子树，内部有自己的 `session` 挂载，其下
 * 消息 / 会话节点 / 收件箱的地址前缀与根那份**不同**。只订根那一份，子空间里
 * 跑起来的会话在界面上永远不动（后端跑得再对也看不见）。
 *
 * 登记是**幂等**的，且允许在订阅启动**之前**调用（三个模块都把它记进自己的
 * 登记表，启动时按表逐份订阅），因此这里不必关心与 `onMounted` 的先后。
 */
watch(
  () => sessions.sessionSpace,
  (dir) => {
    if (!dir) return
    void registerSessionMount(dir)
    void registerTranscriptMount(dir)
    void registerInboxMount(dir)
  },
  { immediate: true },
)

onMounted(async () => {
  // 启动**会话转写同步**（VDFS 变更 → 会话 store；D-025）
  //
  // 实时面的全部内容只经一条通道（`event_bus` 的 `vdfs` 频道 + `vdfs/watch` 登记）。
  // 会话是容器，其下是若干并列的集合（消息 / 子会话 / 记忆 / 工作目录，后续还有
  // 任务列表、请求队列……），地址形状统一为 `<sid>/<集合段>/<项 id>`，**身份就是
  // 地址末段**。两类地址各有归属：`<sid>` = 会话节点自身（运行态 / 资源，归
  // `sessionNodeSync`）；`<sid>/message/<mid>` = 一条消息（归这里）。
  //
  // 消息帧的语义全在字段上，没有操作枚举——顺序是**节点属性**（`ChatNode.seq`），
  // 到达顺序与显示顺序无关。载荷就是帧本身：`delta` 有 ⇒ 零回读追加、`content` 有
  // ⇒ 零回读整条替换、只有 `status` ⇒ 本地已有就零回读迁移状态、`removed` ⇒ 就地
  // 移除；只有「本端缺基线」（身份未知）才回读 `stat` + `read`。
  //
  // 落地目标由本外壳**显式注入**（本模块不认识 Pinia）：判「本地有没有这条消息」
  // （决定要不要回读补基线）、应用一批消息帧、整份重读（resync / 回读失败走
  // loadMessages）。
  //
  // 增量收的是**一批**而不是一条：同步层把 ~48ms 窗口内的增量帧按会话攒批，
  // 一批只做一次 store 提交与一次消息树重建——逐帧提交的代价是 O(历史条数 × 帧数)，
  // 长会话下那才是端到端的主要热点。回读结果**不等窗口**（身份 / 基线的权威来源）。
  void startSessionTranscriptSync({
    hasMessage: (sid, mid) => sessions.hasMessage(sid, mid),
    applyTranscriptMessages: (sid, msgs) => sessions.applyTranscriptMessages(sid, msgs),
    reload: async (sid) => {
      await sessions.loadMessages(sid)
    },
  })

  // 会话清单的 VDFS 订阅同样由外壳接线（store 自己不挂监听器），
  // 于是 HMR / 测试不会叠监听器，订阅的启停也看得见。
  // 现在是 async：订阅前缀（会话挂载目录）要按数据认出来，不是常量。
  await startSessionNodeSync(sessions)

  // 启动**收件箱同步**（排队中的用户消息 → 「待发送」气泡； 步③）
  //
  // 它补的是转写实时面看不见的那一段：消息已入队、空间还在忙，要等当前轮跑完
  // 才被消费。落库后的权威帧由转写消费端照实显示（**无论发送方是谁**），而排队
  // 中这段时间里，「不是我发的消息」（CLI / telegram / 心跳 / 别的进程）连
  // `working` 乐观置位都没有——本模块把队列里有几条、各是什么显示出来。
  //
  // 三个落地动作与转写那一套同构，落点都是 sessions store 的 inbox 部分：
  // 入队（带节点视图）⇒ 加一条，文案取后端 `description`（`待消费 · <预览>`）；
  // 出队 / 取消 / 清空（无载荷）⇒ 回读 `stat` 分辨删除后移除；resync ⇒ 整份重列。
  void startSessionInboxSync({
    applyInboxItem: (sid, entry) => sessions.applyInboxItem(sid, entry),
    removeInboxItem: (sid, iid) => sessions.removeInboxItem(sid, iid),
    reloadInbox: async (sid) => {
      await sessions.refreshInbox(sid)
    },
  })

  // 提示音的设置来源同样由外壳注入（service 不认识 store）：
  // 传的是**取值函数**而非快照，用户改了开关/音量下一声就生效。
  setChimeSettingsSource(() => {
    const s = useSoundSettingsStore()
    return { enabled: (kind) => s.isKindEnabled(kind), volume: s.volume }
  })

  // 恢复全局会话状态：lastWorkdir 供新建会话作默认工作区；
  // sessions store 清单供聊天组件查元数据
  try {
    await getWorkspacePath()
  } catch (err) {
    logger.warn('MainLayout', '恢复全局工作区失败:', err)
  }
  void sessions.refreshList()

  // 首启引导：**只在「该做的事还没做」时自动弹**（判据见 stores/onboarding），
  // 不阻塞任何启动流程——它只是浮层，数据侧照旧自己拉。
  void useOnboardingStore().maybeOpen()

  // 全局键：命令面板（Cmd/Ctrl+K）。挂在这里而不是某个页面里，是因为面板是
  // **跨页面**的——在会话页、在模型页、在子空间里都该按得出来。
  window.addEventListener('keydown', onGlobalKeydown)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onGlobalKeydown)
})
</script>

<style scoped>
.main-layout {
  display: flex;
  width: 100%;
  height: 100vh;
  overflow: hidden;
}
</style>
