import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import path from 'path'

const uiRoot = import.meta.dirname
// 唯一产物源：desktop 的 generate_context!、web 静态部署、CLI 二进制内嵌都读这一个目录。
// Tauri API 是动态 import，browser 分支不会加载，因此一份产物可同时服务三端。
const dist = path.resolve(uiRoot, 'dist')

export default defineConfig({
  root: uiRoot,
  plugins: [vue()],
  resolve: {
    alias: { '@': path.resolve(uiRoot, 'src') }
  },
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    outDir: dist,
    emptyOutDir: true,
    target: ['es2021', 'chrome100', 'safari14'],
    minify: !process.env.TAURI_DEBUG ? 'oxc' : false,
    sourcemap: !!process.env.TAURI_DEBUG,
    rollupOptions: {
      output: {
        manualChunks(id: string): string | undefined {
          const norm = id.replace(/\\/g, '/')
          if (!norm.includes('/node_modules/')) return undefined
          if (norm.includes('/node_modules/@tauri-apps/')) return 'tauri-api'
          if (/\/node_modules\/(@vue\/|vue\/|vue-router\/|pinia\/)/.test(norm)) return 'vue'
          return undefined
        }
      }
    }
  }
})
