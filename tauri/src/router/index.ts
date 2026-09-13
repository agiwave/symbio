import { createRouter, createWebHistory } from 'vue-router'
import MainLayout from '../views/MainLayout.vue'
import WorkbenchView from '../views/WorkbenchView.vue'
import VdfsView from '../views/VdfsView.vue'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/',
      component: MainLayout,
      children: [
        // 首页 = VDFS 会话挂载点（会话是首个迁移到 VDFS 的资源，见
        // docs/design/vdfs-frontend.md §7 S3）：首页即「.vdfs/session」的下一级列表
        { path: '', redirect: () => '/vdfs/session' },
        // VDFS 通用资源页（**嵌入主布局**）：中栏 = 当前目录、右栏 = 按节点 ext
        // 分发的详情渲染器（form / session / text …）；左栏（挂载点导航）由应用
        // 外壳承担，因此全 App 只有一台三栏工作台，页面切换不替换外壳。
        // :mount 可选 = 深链直接进入某挂载点（如 /vdfs/setting、/vdfs/session）。
        {
          path: 'vdfs/:mount?',
          name: 'vdfs',
          component: VdfsView,
          props: (route) => ({
            mount: (route.params.mount as string) || undefined,
            embedded: true,
          }),
        },
        // 统一实体页（全 App 唯一实体页面，机制化配置驱动）：
        // :types = 'all' | 逗号分隔 kind | 单 kind（缺省 all）
        { path: 'entities/:types?', name: 'entities', component: WorkbenchView, props: (route) => ({ typesParam: (route.params.types as string) || undefined }) },
        // 旧专项路由 → redirect 保兼容（书签 / 深链）。
        // S5：这些资源均已迁移到 VDFS（`/vdfs/{mount}`，挂载名与 kind 同名），
        // 故直接指向 VDFS 页，不再经过将被下线的 `/entities/*` 统一实体页。
        { path: 'model-providers', redirect: () => '/vdfs/model' },
        { path: 'mcp', redirect: () => '/vdfs/mcp' },
        { path: 'skill', redirect: () => '/vdfs/skill' },
        { path: 'agent', redirect: () => '/vdfs/agent' },
        // 设置页：已迁移到 VDFS（/vdfs/setting，见 docs/design/vdfs-frontend.md §7 S2）；
        // 旧地址 redirect 保兼容（书签 / 深链）
        { path: 'settings', redirect: () => '/vdfs/setting' }
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
    }
  ]
})

export default router
