import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import path from 'path'

const uiRoot = import.meta.dirname
const desktopDist = path.resolve(uiRoot, '../../apps/desktop/dist')

export default defineConfig({
  root: uiRoot,
  plugins: [vue()],
  resolve: {
    alias: { '@': path.resolve(uiRoot, 'src') }
  },
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    outDir: desktopDist,
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
