import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import path from 'path'

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src')
    }
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true
  },
  build: {
    target: ['es2021', 'chrome100', 'safari14'],
    minify: !process.env.TAURI_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_DEBUG,
    // mermaid 内部的动态导入 chunk 会超过 700KB，这是正常的
    chunkSizeWarningLimit: 1500,
    rollupOptions: {
      output: {
        manualChunks: {
          // 分离 Vue 核心
          'vue': ['vue', 'vue-router', 'pinia'],
          // 分离大型图表库
          'mermaid': ['mermaid'],
          // 分离思维导图库（大型）
          'mindmap': ['@milkdown/plugin-diagram'],
          // 分离 ELK 布局引擎（大型）
          'elk': ['elkjs'],
          // 分离编辑器相关库
          'editor': ['@milkdown/core', '@milkdown/kit', '@milkdown/prose', '@codemirror/view', '@codemirror/state', '@codemirror/commands', '@codemirror/lang-markdown', '@codemirror/language-data', '@codemirror/theme-one-dark', 'marked'],
          // 分离 Milkdown 插件
          'milkdown-plugins': ['@milkdown/plugin-history', '@milkdown/plugin-listener', '@milkdown/plugin-math', '@milkdown/plugin-prism', '@milkdown/preset-commonmark', '@milkdown/preset-gfm'],
          // 分离 Tauri API
          'tauri-api': ['@tauri-apps/api'],
          // 分离 services 代码
          'services': ['@/services/model', '@/services/plugin', '@/services/session', '@/services/config'],
          // 分离 stores 代码
          'stores': ['@/stores/explorer']
        }
      }
    }
  }
})
