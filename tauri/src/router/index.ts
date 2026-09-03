import { createRouter, createWebHistory } from 'vue-router'
import MainLayout from '../views/MainLayout.vue'
import SessionView from '../views/SessionView.vue'
import FileViewerWindow from '../views/FileViewerWindow.vue'
import ResourceManagerView from '../views/ResourceManagerView.vue'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/',
      component: MainLayout,
      children: [
        { path: '', name: 'session', component: SessionView },
        // 统一资源浏览器：:types = 'all' | 逗号分隔 kind | 单 kind（缺省 all）
        { path: 'resources/:types?', name: 'resources', component: ResourceManagerView, props: (route) => ({ typesParam: (route.params.types as string) || undefined }) },
        // 旧专项路由 → redirect 保兼容（书签 / 深链）
        { path: 'model-providers', redirect: () => '/resources/model' },
        { path: 'mcp', redirect: () => '/resources/mcp' },
        { path: 'skill', redirect: () => '/resources/skill' },
        { path: 'agent', redirect: () => '/resources/agent' },
        { path: 'settings', name: 'settings', component: () => import('../components/SettingsPage.vue') }
      ]
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
