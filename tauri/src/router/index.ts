import { createRouter, createWebHistory } from 'vue-router'
import MainLayout from '../views/MainLayout.vue'
import FileViewerWindow from '../views/FileViewerWindow.vue'
import WorkbenchView from '../views/WorkbenchView.vue'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/',
      component: MainLayout,
      children: [
        // 会话页 = 统一实体页的 session 实例（机制列表 + 聊天工作区 editor）
        { path: '', name: 'session', component: WorkbenchView, props: () => ({ typesParam: 'session' }) },
        // 统一实体页（全 App 唯一实体页面，机制化配置驱动）：
        // :types = 'all' | 逗号分隔 kind | 单 kind（缺省 all）
        { path: 'entities/:types?', name: 'entities', component: WorkbenchView, props: (route) => ({ typesParam: (route.params.types as string) || undefined }) },
        // 旧专项路由 → redirect 保兼容（书签 / 深链）
        { path: 'model-providers', redirect: () => '/entities/model' },
        { path: 'mcp', redirect: () => '/entities/mcp' },
        { path: 'skill', redirect: () => '/entities/skill' },
        { path: 'agent', redirect: () => '/entities/agent' },
        // 设置页：同一 WorkbenchView 的 setting 实例（分区清单来自后端 setting/entities/list）
        { path: 'settings', name: 'settings', component: WorkbenchView, props: () => ({ typesParam: 'setting' }) }
      ]
    },
    {
      // 统一实体页（container 模式，全局页面级推入，整页替换主布局）：
      // 与主界面同构的 侧边栏（ProviderInfo.container_kinds 下发的子类别）
      // + 列表 + 详情；协议为同一套 entities/*（payload.container）。
      // :kind = 容器所属 provider kind；:id = 容器条目 id
      path: '/container/:kind/:id/entities',
      name: 'container-entities',
      component: WorkbenchView,
      props: (route) => ({
        containerKind: route.params.kind as string,
        containerId: route.params.id as string,
      })
    },
    {
      // agent 兼容别名（深链保兼容）：/agent/:agentId/entities
      path: '/agent/:agentId/entities',
      name: 'agent-entities',
      component: WorkbenchView,
      props: (route) => ({
        containerKind: 'agent',
        containerId: route.params.agentId as string,
      })
    },
    {
      // 文件查看器（独立窗口）
      path: '/file-viewer',
      name: 'file-viewer',
      component: FileViewerWindow
    }
  ]
})

export default router
