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
import { startSessionTranscriptSync } from '@/stores/sessionTranscriptSync'
import { startSessionNodeSync } from '@/stores/sessionNodeSync'
import { setChimeSettingsSource } from '@/services/completionChime'
import { getWorkspacePath } from '@/services/home'
import { useSessionsStore } from '@/stores/sessions'
import { useSoundSettingsStore } from '@/stores/soundSettings'
import { logger } from '@/utils/logger'
import Toast from '@/components/common/Toast.vue'

onMounted(async () => {
  // 启动**会话转写同步**（VDFS 变更 → 会话 store；ADR-025）
  //
  // 实时面的全部内容只经一条通道（`event_bus` 的 `vdfs` 频道 + `vdfs/watch` 登记），
  // 两类地址共用它：`<sid>` = 会话节点（运行态 / 资源，`stat` 回读收敛）；
  // `<sid>/消息/<mid>` = 一条消息（`delta` 有 ⇒ 零回读追加，无 ⇒ 回读身份与基线，
  // `deleted` ⇒ 就地移除）。语义全在字段上，没有操作枚举——顺序是**节点属性**
  // （`ChatMessage.seq`），到达顺序与显示顺序无关。
  //
  // 落地目标由本外壳**显式注入**（本模块不认识 Pinia）：判「本地有没有这条消息」
  // （决定增量走零回读还是回读）、应用一批消息帧、应用一帧会话运行态、
  // 整份重读（resync / 回读失败走 loadMessages）。
  //
  // 增量收的是**一批**而不是一条：同步层把 ~48ms 窗口内的增量帧按会话攒批，
  // 一批只做一次 store 提交与一次消息树重建——逐帧提交的代价是 O(历史条数 × 帧数)，
  // 长会话下那才是端到端的主要热点。回读结果**不等窗口**（身份 / 基线的权威来源）。
  const sessions = useSessionsStore()
  void startSessionTranscriptSync({
    hasMessage: (sid, mid) => sessions.hasMessage(sid, mid),
    applyTranscriptMessages: (sid, msgs) => sessions.applyTranscriptMessages(sid, msgs),
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
