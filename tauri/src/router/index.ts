import { createRouter, createWebHistory } from 'vue-router'
import MainLayout from '../views/MainLayout.vue'
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
        // 旧统一实体页（S5 已下线）→ redirect 保兼容（书签 / 深链）：
        // :types = 'all' | 逗号分隔 kind | 单 kind（缺省 all）。
        // 单 kind 直达对应 VDFS 挂载点（挂载名与 kind 同名）；其余回落首页挂载点。
        {
          path: 'entities/:types?',
          redirect: (to) => {
            const first = String(to.params.types ?? '').split(',')[0]
            return first && first !== 'all' ? `/vdfs/${first}` : '/vdfs/session'
          },
        },
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
    // 容器实体页（`/container/:kind/:id/entities`、`/agent/:agentId/entities`）
    // 已随 S5 下线：容器的「管理内部实体」能力由 VDFS 的
    // `<id>/<子类别标签>/<条目>` 寻址承担（见 docs/design/vdfs-frontend.md §7 S7），
    // 整页推入的容器页不再是必需入口，故不再保留路由。
  ]
})

export default router
