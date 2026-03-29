<template>
  <div class="app-container">
    <header class="app-header">
      <h1>🌊 Symbio 插件系统</h1>
    </header>

    <main class="app-main">
      <!-- 诊断组件 -->
      <Diagnostic />
    </main>
  </div>
</template>

<script setup lang="ts">
import Diagnostic from './components/Diagnostic.vue'
</script>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

body {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif;
  background: #f5f5f5;
  color: #333;
}

.app-container {
  display: flex;
  flex-direction: column;
  height: 100vh;
}

.app-header {
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: white;
  padding: 1rem 2rem;
  box-shadow: 0 2px 10px rgba(0,0,0,0.1);
}

.app-header h1 {
  font-size: 1.5rem;
  font-weight: 600;
}

.app-main {
  display: flex;
  flex: 1;
  overflow: hidden;
}

.plugin-sidebar {
  width: 300px;
  background: white;
  border-right: 1px solid #e0e0e0;
  overflow-y: auto;
}

.plugin-workspace {
  flex: 1;
  padding: 2rem;
  overflow-y: auto;
  background: #fafafa;
}

.empty-workspace {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: #999;
}

.empty-workspace h2 {
  margin-bottom: 0.5rem;
  color: #666;
}

.workspace-content {
  margin-top: 1.5rem;
}

.no-input-hint {
  background: white;
  padding: 1.5rem;
  border-radius: 8px;
  margin-bottom: 1.5rem;
  box-shadow: 0 2px 8px rgba(0,0,0,0.05);
}

.no-input-hint button {
  margin-top: 1rem;
  padding: 0.75rem 1.5rem;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: white;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  font-size: 1rem;
  transition: transform 0.2s;
}

.no-input-hint button:hover:not(:disabled) {
  transform: translateY(-2px);
}

.no-input-hint button:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
</style>
