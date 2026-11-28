import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import path from 'path'

const uiRoot = import.meta.dirname
// 唯一产物源：web 静态部署与 CLI 二进制内嵌都读这一个目录；桌面原生窗口加载的
// CLI serve 提供的就是这份内嵌产物。三端共用一份，不再有 IPC 专用 chunk。
const dist = path.resolve(uiRoot, 'dist')

export default defineConfig({
  root: uiRoot,
  plugins: [vue()],
  resolve: {
    alias: { '@': path.resolve(uiRoot, 'src') }
  },
  clearScreen: false,
  // 开发态：Vite 提供前端热更新（HMR），后端由 `symbio-cli serve` 单独提供。
  // 前端 transport 走同源 `/api`，这里把 /api（含 WS）代理到本机 CLI Server，
  // Tauri 原生窗口在 dev 时加载 Vite 地址，即旧 tauri dev 的热更新体验。
  // SYMBIO_SERVER_PORT 与 CLI serve 共用同一个变量，避免两处端口漂移。
  server: {
    port: 1420,
    strictPort: true,
    proxy: {
      '/api': {
        target: `http://127.0.0.1:${process.env.SYMBIO_SERVER_PORT ?? '9231'}`,
        // 必须 false：Server 的 loopback 校验要求 Host 与 Origin 同源。
        // changeOrigin:true 只改 Host 不改 Origin ⇒ POST 带 Origin=localhost:1420、
        // Host=127.0.0.1:9231，被判 cross-origin（切换 homedir 等 POST 全失败）。
        // false 时两者都是 localhost:1420，同源通过；服务端只按 Host 判 loopback，
        // 接受 localhost，不依赖 Host 指向真实目标。
        changeOrigin: false,
        ws: true,
        // 窗口关闭 / CLI 重启时 WS 对端会先发 RST，代理读到 ECONNRESET/EPIPE。
        // 这类是良性的对端断连，吞掉即可；否则 Vite 会把 unhandled 'error' 打成
        // "[vite] ws proxy error" 甚至拖垮 dev server。非良性错误照常打日志。
        configure: (proxy) => {
          proxy.on('error', (err: NodeJS.ErrnoException) => {
            if (err.code === 'ECONNRESET' || err.code === 'EPIPE' || err.code === 'ECONNABORTED') return
            console.warn('[vite /api proxy]', err.code ?? err.message)
          })
        },
      },
    },
  },
  build: {
    outDir: dist,
    emptyOutDir: true,
    target: ['es2021', 'chrome100', 'safari14'],
    minify: 'oxc',
    rollupOptions: {
      output: {
        manualChunks(id: string): string | undefined {
          const norm = id.replace(/\\/g, '/')
          if (!norm.includes('/node_modules/')) return undefined
          if (/\/node_modules\/(@vue\/|vue\/|vue-router\/|pinia\/)/.test(norm)) return 'vue'
          return undefined
        }
      }
    }
  }
})
