<!--
  MainLayout — 应用根布局（全局单例职责，不含任何页面/导航知识）

  三栏工作台由各路由页面自持（VdfsView 嵌入唯一的 VdfsWorkbench 控件），
  本文件不再有外壳三栏；只承担**跨页面全局**职责：

  - 路由出口（RouterView）；
  - 全局浮动消息浮层（Toast，状态来自 useToast 单例，全 App 仅渲染一次）；
  - 全局初始化：VDFS 转写同步 + 工作区恢复（须先于页面挂载）。
-->
<template>
  <div class="main-layout">
    <!-- 不加 :key：路由变化由 VdfsView 依 route.params 换算数据地址，
         控件（VdfsWorkbench）按绑定的数据地址自行重载 -->
    <RouterView />
    <!-- 全局浮动消息浮层 -->
    <Toast />
  </div>
</template>

<script setup lang="ts">
import { onMounted } from 'vue'
import { RouterView } from 'vue-router'
import { startTranscriptStream } from '@/services/transcriptStream'
import { startSessionNodeSync } from '@/stores/sessionNodeSync'
import { setChimeSettingsSource } from '@/services/completionChime'
import { getWorkspacePath } from '@/services/home'
import { useSessionsStore } from '@/stores/sessions'
import { useSoundSettingsStore } from '@/stores/soundSettings'
import { logger } from '@/utils/logger'
import Toast from '@/components/common/Toast.vue'

onMounted(async () => {
  // 启动**转写实时流**（`worker/session/stream` → 会话 store）
  //
  // 实时面的全部内容只经这一条流，两种帧共用一个 `seq` 计数器：
  // `transcript_event` 帧 = 一条 `ChatMessage`（语义全在字段上——delta 追加 /
  // content 替换 / status=removed 移除）；`transcript_session` 帧 = 会话节点的
  // 全量视图（运行态 / 结局 / 错误 / 告警）。
  //
  // 会话运行态之所以也在这条流上，是因为它必须与它那一轮的消息**共用 `seq` 空间**：
  // 于是「会话报不忙」到达时，本轮全部终态帧必然已落地——一条结构性保证，取代了
  // 原先"两条通道各有一个泵任务、到达顺序靠调度巧合"的假设。资源变更（创建 / 删除 /
  // 改名 / 标题）仍走 VDFS，由 `startSessionNodeSync` 收敛——那里**不携带节点快照**，
  // 快照只有本流（有序）与 `list` / `stat` 回读两个来源。
  //
  // 落地目标由本外壳**显式注入**（`transcriptStream` 是 service，不认识 Pinia）；
  // 三个动作：应用**一批**消息帧、应用一帧会话运行态、整份重读（序号缺口 / resync
  // 走 loadMessages）。「帧该做什么」由落地目标按字段判定，不按操作分派到不同的落地口。
  //
  // 消息收的是**一批**而不是一条：`transcriptStream` 把 ~48ms 窗口内的帧按会话攒批，
  // 一批只做一次 store 提交与一次消息树重建——逐帧提交的代价是 O(历史条数 × 帧数)，
  // 长会话下那才是端到端的主要热点（帧的 `seq` 语义不变，逐帧仍过缺口检测）。
  // 运行态帧**不等窗口**：它到达时先把同会话待落地的消息帧冲掉，再交付节点视图。
  const sessions = useSessionsStore()
  void startTranscriptStream({
    messages: (sid, msgs) => sessions.applyTranscriptMessages(sid, msgs),
    applySessionState: (sid, node) => sessions.applySessionState(sid, node),
    reload: async (sid) => {
      await sessions.loadMessages(sid)
    },
  })

  // 会话清单的 VDFS 订阅同样由外壳接线（store 自己不挂监听器），
  // 于是 HMR / 测试不会叠监听器，订阅的启停也看得见。
  // 现在是 async：订阅前缀（会话挂载目录）要按数据认出来，不是常量。
  await startSessionNodeSync(sessions)

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
  void useSessionsStore().refreshList()
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
