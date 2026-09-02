import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import path from 'path'

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: {
      // ESM 配置（package.json "type": "module"）下无 __dirname，用 import.meta.dirname
      '@': path.resolve(import.meta.dirname, './src')
    }
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true
  },
  build: {
    target: ['es2021', 'chrome100', 'safari14'],
    // vite 8 起不再内置 esbuild，压缩器默认改为 oxc
    minify: !process.env.TAURI_DEBUG ? 'oxc' : false,
    sourcemap: !!process.env.TAURI_DEBUG,
    // mermaid 内部的动态导入 chunk 会超过 700KB，这是正常的
    chunkSizeWarningLimit: 1500,
    rollupOptions: {
      output: {
        // vite 8（rolldown 内核）只支持函数形式的 manualChunks，对象形式会直接报错
        manualChunks(id: string): string | undefined {
          const norm = id.replace(/\\/g, '/')
          if (norm.includes('/node_modules/')) {
            if (norm.includes('/node_modules/mermaid/')) return 'mermaid'
            if (norm.includes('/node_modules/elkjs/')) return 'elk'
            // 分离思维导图/ELK 相关大型依赖
            if (norm.includes('/node_modules/@milkdown/plugin-diagram/')) return 'mindmap'
            // 编辑器核心：milkdown 核心 + codemirror + marked
            if (/\/node_modules\/(@milkdown\/(core|kit|prose)\/|@codemirror\/|marked\/)/.test(norm)) {
              return 'editor'
            }
            // 其余 milkdown 插件
            if (norm.includes('/node_modules/@milkdown/')) return 'milkdown-plugins'
            if (norm.includes('/node_modules/@tauri-apps/api/')) return 'tauri-api'
            // Vue 核心
            if (
              /\/node_modules\/(@vue\/|vue\/|vue-router\/|pinia\/)/.test(norm)
            ) {
              return 'vue'
            }
            return undefined
          }
          // 分离 services / stores 业务代码
          if (/\/src\/services\/(model|plugin|session|config)\./.test(norm)) return 'services'
          if (norm.includes('/src/stores/explorer')) return 'stores'
          return undefined
        }
      }
    }
  }
})
