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
  // 启动**转写实时流**（`worker/session/stream` → 会话消息 store）
  //
  // 消息本体的全部实时显示只经这一条流（`NodeEvent`：帧 = 一条 `ChatMessage`，
  // 语义全在字段上——delta 追加 / content 替换 / status=removed 移除）。会话运行态
  // （`<根>/session/<sid>` 叶子）仍走 VDFS 变更，由 `startSessionNodeSync` 收敛——
  // 两者分工明确，同一份真相不会被写两次。
  //
  // 落地目标由本外壳**显式注入**（`transcriptStream` 是 service，不认识 Pinia）；
  // 只有两个动作：应用**一批**消息帧、整份重读（序号缺口 / resync 走 loadMessages）。
  // 「帧该做什么」由落地目标按字段判定，不再按操作分派到不同的落地口。
  //
  // 收的是**一批**而不是一条：`transcriptStream` 把 ~48ms 窗口内的帧按会话攒批，
  // 一批只做一次 store 提交与一次消息树重建——逐帧提交的代价是 O(历史条数 × 帧数)，
  // 长会话下那才是端到端的主要热点（帧的 `seq` 语义不变，逐帧仍过缺口检测）。
  const sessions = useSessionsStore()
  void startTranscriptStream({
    messages: (sid, msgs) => sessions.applyTranscriptMessages(sid, msgs),
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
