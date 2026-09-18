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

const app = createApp(App)

app.use(pinia)
app.use(router)

// 在挂载前应用已持久化的外观设置（主题 / 字体大小）
useAppearanceStore().apply()

app.mount('#app')

// 启动期预读网关出站配置（Promise 已缓存；首次出站请求前必已完成，消除竞态）。
// 始终走原生 invoke 读取，不受出站协议切换影响，无「鸡生蛋」问题。
void initGatewayTransport()
