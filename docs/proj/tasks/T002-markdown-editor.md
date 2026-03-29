# T002: Markdown 编辑器集成

## 基本信息

| 属性 | 值 |
|------|-----|
| 任务ID | T002 |
| 标题 | Markdown 编辑器集成 |
| 阶段 | Phase 1: 核心能力 (MVP) |
| 优先级 | P0 |
| 预估工时 | 24h |
| 状态 | pending |
| 依赖 | T001 |

## 任务描述

集成一个支持代码块执行的 Markdown 编辑器，用户可以在文档中编写和执行生信分析代码。

## 验收标准

- [ ] Markdown 实时预览功能正常
- [ ] 代码块语法高亮显示
- [ ] 代码块执行按钮可点击
- [ ] 执行结果显示在代码块下方
- [ ] 支持基础 Markdown 语法

## 技术要求

### 编辑器选型

**候选方案对比：**

| 方案 | 优点 | 缺点 | 推荐度 |
|------|------|------|--------|
| Milkdown | 插件化、现代、WYSIWYG | 文档较少 | ⭐⭐⭐⭐ |
| Codemirror | 成熟稳定、高度可定制 | 需要自己实现预览 | ⭐⭐⭐⭐ |
| TipTap | 块级编辑、协作支持 | 学习曲线高 | ⭐⭐⭐ |

**推荐方案：Milkdown**
- 插件化架构，易于扩展
- 支持 WYSIWYG 编辑
- 内置 Markdown 解析

### 核心功能

```typescript
// 编辑器配置
interface EditorConfig {
  // 基础功能
  syntaxHighlighting: boolean;
  livePreview: boolean;
  
  // 代码块功能
  codeBlockExecution: boolean;
  supportedLanguages: ['bash', 'r', 'python'];
  
  // 扩展功能
  imageUpload: boolean;
  tableEditing: boolean;
  dragAndDrop: boolean;
}
```

### 代码块组件

```vue
<!-- CodeBlock.vue -->
<template>
  <div class="code-block">
    <div class="code-header">
      <span class="language">{{ language }}</span>
      <button @click="execute" :disabled="executing">
        {{ executing ? '执行中...' : '执行' }}
      </button>
    </div>
    <div class="code-content">
      <code-editor v-model="code" :language="language" />
    </div>
    <div class="code-result" v-if="result">
      <pre>{{ result }}</pre>
    </div>
  </div>
</template>
```

## 子任务

1. **编辑器基础集成** (6h)
   - 集成 Milkdown 编辑器
   - 配置基础 Markdown 插件
   - 实现实时预览

2. **代码块语法高亮** (4h)
   - 集成 Prism.js 或 Shiki
   - 支持 Bash/R/Python 语法
   - 自定义代码块样式

3. **代码块执行按钮** (4h)
   - 设计执行按钮 UI
   - 连接执行 API
   - 处理执行状态

4. **结果显示组件** (4h)
   - 设计结果展示区域
   - 支持文本和图片输出
   - 错误信息高亮

5. **编辑器工具栏** (4h)
   - 基础格式化工具
   - 插入代码块快捷方式
   - 插入图片功能

6. **测试和优化** (2h)
   - 编写单元测试
   - 性能优化

## 依赖

- T001: 项目基础架构搭建

## 输出物

- Markdown 编辑器组件
- 代码块执行组件
- 编辑器配置文档

## 风险

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| Milkdown 文档不足 | 中 | 预留时间调研 |
| 大文件性能问题 | 低 | 实现虚拟滚动 |

## 备注

编辑器是用户的核心工作区，需要注重用户体验和性能。
