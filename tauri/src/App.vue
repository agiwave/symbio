<template>
  <div class="app-container">
    <RouterView />
  </div>
</template>

<script setup lang="ts">
import { RouterView } from 'vue-router'
</script>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

:root {
  --color-primary: #667eea;
  --color-primary-dark: #764ba2;
  --color-bg: #f5f5f5;
  --color-surface: #ffffff;
  --color-border: #e0e0e0;
  --color-text: #333333;
  --color-text-secondary: #666666;
  --color-text-muted: #999999;
  
  --sidebar-width: 48px;
  --panel-width: 280px;
  --header-height: 48px;
}

html, body {
  height: 100%;
  overflow: hidden;
}

body {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif;
  background: var(--color-bg);
  color: var(--color-text);
  /* 用 rem 参照 <html> 根字号（16px），使字号档位（小/中/大）能整体缩放界面文本 */
  font-size: 0.875rem;
  line-height: 1.5;
}

.app-container {
  display: flex;
  height: 100vh;
  width: 100vw;
  overflow: hidden;
}

/* 滚动条样式 */
::-webkit-scrollbar {
  width: 8px;
  height: 8px;
}

::-webkit-scrollbar-track {
  background: transparent;
}

::-webkit-scrollbar-thumb {
  background: #ccc;
  border-radius: 4px;
}

::-webkit-scrollbar-thumb:hover {
  background: #bbb;
}

:root[data-theme="dark"] {
  --color-primary: #818cf8;
  --color-primary-dark: #6366f1;
  --color-bg: #1e1e2e;
  --color-surface: #27273a;
  --color-border: #3f3f5a;
  --color-text: #e2e8f0;
  --color-text-secondary: #94a3b8;
  --color-text-muted: #64748b;
}
</style>
