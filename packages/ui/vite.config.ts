import { createLogger, defineConfig, type Logger } from 'vite'
import vue from '@vitejs/plugin-vue'
import path from 'path'

const uiRoot = import.meta.dirname
// 唯一产物源：web 静态部署与 CLI 二进制内嵌都读这一个目录；桌面原生窗口加载的
// CLI serve 提供的就是这份内嵌产物。三端共用一份，不再有 IPC 专用 chunk。
const dist = path.resolve(uiRoot, 'dist')

// 关闭 Tauri 窗口时 CLI 子进程被终止，Vite 的 /api WS 代理读到对端 RST，
// Vite 内部无条件注册的错误处理器会把它打成 "[vite] ws proxy error"。
// 这是良性的退出噪音：用自定义 logger 只过滤「代理错误 + 对端断连错误码」
// 这一组合，其它错误（含代理的 502、编译错误等）照常输出。
// 注意：在 proxy.configure 里 on('error') 拦不住这条日志——EventEmitter 会
// 复制监听器列表逐个调用，Vite 自己的处理器仍然会打印。
const benignProxyCodes = new Set(['ECONNRESET', 'EPIPE', 'ECONNABORTED'])
const proxyErrorLabels = ['http proxy error:', 'ws proxy error:', 'ws proxy socket error:']

function isBenignProxyDisconnect(message: string, error: unknown): boolean {
  if (!proxyErrorLabels.some((label) => message.includes(label))) return false
  const code = (error as NodeJS.ErrnoException | undefined)?.code
  return code !== undefined && benignProxyCodes.has(code)
}

const defaultLogger = createLogger()
const logger: Logger = {
  info: (...args) => defaultLogger.info(...args),
  warn: (...args) => defaultLogger.warn(...args),
  warnOnce: (...args) => defaultLogger.warnOnce(...args),
  error: (message, options) => {
    if (isBenignProxyDisconnect(message, options?.error)) return
    defaultLogger.error(message, options)
  },
  clearScreen: (...args) => defaultLogger.clearScreen(...args),
  hasErrorLogged: (...args) => defaultLogger.hasErrorLogged(...args),
  get hasWarned() {
    return defaultLogger.hasWarned
  },
}

export default defineConfig({
  root: uiRoot,
  plugins: [vue()],
  customLogger: logger,
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
