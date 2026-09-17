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
import { startTranscriptSync } from '@/services/vdfsTranscriptSync'
import { getWorkspacePath } from '@/services/home'
import { useSessionsStore } from '@/stores/sessions'
import { logger } from '@/utils/logger'
import Toast from '@/components/common/Toast.vue'

onMounted(async () => {
  // 启动转写同步（VDFS 变更 → 会话消息 store）
  //
  // 会话的**全部**实时显示都经这一条频道（`kind = "vdfs"`）：
  // 消息本体（`.vdfs/session/<sid>/消息/<mid>`）由本模块收敛，
  // 会话运行态（`.vdfs/session/<sid>`）由 sessions store 自己的订阅作用域收敛。
  // 两者按**地址**分流，互不重叠——若消息被两条通道各写一次，流式文本会叠字。
  startTranscriptSync()

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
