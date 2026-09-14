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
        { path: '', redirect: () => '/vdfs' },
        // 首页 = `.vdfs` 目录本身；`/vdfs/<dir…>` = `.vdfs/<dir…>` 地址页
        // （push 出来的，如会话内部）。两者都是同一个 VdfsView：它把浏览器地址
        // 换算成**数据地址**（`.vdfs` / `.vdfs/<dir…>`）绑定给唯一的三栏控件
        // VdfsWorkbench——数据地址与浏览器地址是两个概念，路由只做承载。
        // :dir 可选 = `.vdfs` 之下的相对路径（可多级，如 /vdfs/session/<id>/工作目录），
        // 深链与「浏览内部」push 出来的地址页都由本路由承接。
        { path: 'vdfs/:dir(.*)*', name: 'vdfs', component: VdfsView },
        // 旧统一实体页（S5 已下线）→ redirect 保兼容（书签 / 深链）：
        // :types = 'all' | 逗号分隔 kind | 单 kind（缺省 all）。
        // 单 kind 直达对应 VDFS 挂载点（挂载名与 kind 同名）；其余回落首页挂载点。
        {
          path: 'entities/:types?',
          redirect: (to) => {
            const first = String(to.params.types ?? '').split(',')[0]
            return first && first !== 'all' ? `/vdfs/${first}` : '/vdfs'
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
