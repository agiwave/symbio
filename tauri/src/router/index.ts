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
        // 旧地址的兼容 redirect（书签 / 深链）：指向同名挂载的 VDFS 页。
        // 机制与沿革见 docs/design/vdfs.md，此处只保留 URL 字面量。
        {
          path: 'entities/:types?',
          redirect: (to) => {
            const first = String(to.params.types ?? '').split(',')[0]
            return first && first !== 'all' ? `/vdfs/${first}` : '/vdfs'
          },
        },
        // 同理：各专项页面的旧地址一律指向 `/vdfs/<挂载名>`（挂载名与 kind 同名）
        { path: 'model-providers', redirect: () => '/vdfs/model' },
        { path: 'mcp', redirect: () => '/vdfs/mcp' },
        { path: 'skill', redirect: () => '/vdfs/skill' },
        { path: 'agent', redirect: () => '/vdfs/agent' },
        { path: 'settings', redirect: () => '/vdfs/setting' }
      ]
    },
    // 容器内部的整页入口不再需要：内部结构由 VDFS 的
    // `<id>/<子类别>/<条目>` 寻址承担（钻入 = push 一个地址页，见 docs/design/vdfs.md）。
  ]
})

export default router
