import { createApp } from 'vue'
import { pinia } from './stores'
import router from './router'
import './styles/tokens.css'
import './styles/base.css'
import './styles/controls.css'
// 内容呈现（Markdown 正文排版 / JSON 语法着色）：全局而非组件 scoped，
// 理由见该文件头注释（类名契约 + 特异性可控）
import './styles/markdown.css'
import App from './App.vue'
import { useAppearanceStore } from './stores/appearance'
import { initGatewayTransport } from './services/plugin'
import { ensureVdfsRoot } from './services/vdfs'

const app = createApp(App)

app.use(pinia)
app.use(router)

// 在挂载前应用已持久化的外观设置（主题 / 字体大小）
useAppearanceStore().apply()

// 挂载前引导 VDFS 根锚点（`vdfs/root`，见 schemas/vdfsRoot）：路由换算
// （schemas/vdfsAddress）读它，必须先于首次导航就绪——顶层 await 消除
// 「深链接直达 /vdfs/… 时锚点未就绪」的窗口。失败不阻断启动（降级为
// 无虚拟半，services/vdfs 里已记日志）。
await ensureVdfsRoot()

app.mount('#app')

// 启动期预读网关出站配置（Promise 已缓存；首次出站请求前必已完成，消除竞态）。
// 始终走原生 invoke 读取，不受出站协议切换影响，无「鸡生蛋」问题。
void initGatewayTransport()
