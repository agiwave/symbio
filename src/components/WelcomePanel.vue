<template>
  <div class="welcome-panel">
    <div class="welcome-content">
      <h1>🌊 Symbio</h1>
      <p class="tagline">在做中学：生信分析的互动学习平台</p>
      
      <div class="features">
        <div class="feature-card" @click="$emit('start-workspace')">
          <div class="feature-icon">📝</div>
          <h3>开始分析</h3>
          <p>创建新的分析项目，或继续之前的工作</p>
        </div>
        
        <div class="feature-card">
          <div class="feature-icon">📚</div>
          <h3>学习流程</h3>
          <p>浏览 RNA-seq、ChIP-seq 等分析模板</p>
        </div>
        
        <div class="feature-card">
          <div class="feature-icon">🔄</div>
          <h3>复用流程</h3>
          <p>使用已保存的分析流程，快速开始</p>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
defineEmits<{
  'start-workspace': []
}>()
</script>

<style scoped>
.welcome-panel {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  flex: 1;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
}

.welcome-content {
  text-align: center;
  color: white;
}

.welcome-content h1 {
  font-size: 3rem;
  margin-bottom: 0.5rem;
}

.tagline {
  font-size: 1.25rem;
  opacity: 0.9;
  margin-bottom: 3rem;
}

.features {
  display: flex;
  gap: 1.5rem;
}

.feature-card {
  background: rgba(255, 255, 255, 0.15);
  backdrop-filter: blur(10px);
  border-radius: 12px;
  padding: 1.5rem;
  width: 200px;
  cursor: pointer;
  transition: all 0.3s ease;
}

.feature-card:hover {
  background: rgba(255, 255, 255, 0.25);
  transform: translateY(-4px);
}

.feature-icon {
  font-size: 2.5rem;
  margin-bottom: 0.5rem;
}

.feature-card h3 {
  font-size: 1.1rem;
  margin-bottom: 0.5rem;
}

.feature-card p {
  font-size: 0.875rem;
  opacity: 0.8;
}
</style>
