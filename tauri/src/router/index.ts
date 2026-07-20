import { createRouter, createWebHistory } from 'vue-router'
import MainLayout from '../views/MainLayout.vue'
import SessionView from '../views/SessionView.vue'
import FileViewerWindow from '../views/FileViewerWindow.vue'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/',
      component: MainLayout,
      children: [
        { path: '', name: 'session', component: SessionView },
        { path: 'model-providers', name: 'model-providers', component: () => import('../views/ModelProvidersView.vue') },
        { path: 'mcp', name: 'mcp', component: () => import('../views/McpView.vue') },
        { path: 'skill', name: 'skill', component: () => import('../views/SkillView.vue') },
        { path: 'agent', name: 'agent', component: () => import('../views/AgentView.vue') },
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
