import { createApp } from 'vue'
import { pinia } from './stores'
import router from './router'
import './styles/tokens.css'
import './styles/base.css'
import App from './App.vue'
import { useAppearanceStore } from './stores/appearance'

const app = createApp(App)

app.use(pinia)
app.use(router)

// 在挂载前应用已持久化的外观设置（主题 / 字体大小）
useAppearanceStore().apply()

app.mount('#app')
