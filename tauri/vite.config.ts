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
    rollupOptions: {
      output: {
        // vite 8（rolldown 内核）只支持函数形式的 manualChunks，对象形式会直接报错。
        //
        // 只分离**真实存在**的第三方库：Vue 运行时与 Tauri IPC 各自成块，
        // 二者更新频率与业务代码不同，分开有利于缓存命中。
        // 业务代码（services / stores / components）不做人工分块——应用本体由
        // vite 按动态导入自动切分；手写路径规则只会随文件移动而腐烂
        // （此前那份规则里 mermaid / elkjs / milkdown / codemirror /
        // `src/stores/explorer` 指向的代码早已删除，是纯残留）。
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
