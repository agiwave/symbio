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
        { path: 'model-providers', name: 'model-providers', component: ResourceManagerView, props: { resourceType: 'model' } },
        { path: 'mcp', name: 'mcp', component: ResourceManagerView, props: { resourceType: 'mcp' } },
        { path: 'skill', name: 'skill', component: ResourceManagerView, props: { resourceType: 'skill' } },
        { path: 'agent', name: 'agent', component: ResourceManagerView, props: { resourceType: 'agent' } },
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
