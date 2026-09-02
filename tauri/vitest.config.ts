import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'
import path from 'path'

/**
 * Vitest 配置 —— 前端单元测试
 *
 * 范围约定（与 CI 门禁对齐）：
 * - **纯逻辑层**（utils / composables / services 协议编解码 / schemas）：必须覆盖
 * - **组件测试**：@vue/test-utils + happy-dom（MessageNode.spec.ts 等），
 *   DOM 用例在文件头以 `// @vitest-environment happy-dom` 标注
 *
 * 运行：`npm test`（单次）/ `npm run test:watch`（监视模式）
 */
export default defineConfig({
  // .vue 单文件组件编译（组件测试挂载 MessageNode 等必需）
  plugins: [vue()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src')
    }
  },
  test: {
    // 默认 node 环境覆盖纯逻辑层；DOM 用例按文件标注 happy-dom
    environment: 'node',
    include: ['src/**/*.{test,spec}.ts'],
    // 被测代码与测试同目录（__tests__）或 *.spec.ts 命名
    globals: false
  }
})