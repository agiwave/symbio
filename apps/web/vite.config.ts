import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import path from 'path'

// Web 交付物复用 packages/ui 的源码与 index.html；产物落在本包 dist，可静态部署。
// 与 desktop 的唯一差别：不注入 Tauri IPC，transport 由 ui 的 browser 分支选 HTTP。
const uiRoot = path.resolve(import.meta.dirname, '../../packages/ui')

export default defineConfig({
  root: uiRoot,
  plugins: [vue()],
  resolve: {
    alias: { '@': path.resolve(uiRoot, 'src') }
  },
  build: {
    outDir: path.resolve(import.meta.dirname, 'dist'),
    emptyOutDir: true,
    target: ['es2021', 'chrome100', 'safari14'],
    minify: !process.env.TAURI_DEBUG ? 'oxc' : false,
    rollupOptions: {
      output: {
        // 浏览器交付物不引入 Tauri IPC 分块（browser 分支不加载 @tauri-apps）
        manualChunks(id: string): string | undefined {
          const norm = id.replace(/\/g, '/')
          if (!norm.includes('/node_modules/')) return undefined
          if (/\/node_modules\/(@vue\/|vue\/|vue-router\/|pinia\/)/.test(norm)) return 'vue'
          return undefined
        }
      }
    }
  }
})
