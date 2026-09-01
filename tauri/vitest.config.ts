import { defineConfig } from 'vitest/config'
import path from 'path'

/**
 * Vitest 配置 —— 前端单元测试
 *
 * 范围约定（与 CI 门禁对齐）：
 * - **纯逻辑层**（utils / composables / services 协议编解码 / schemas）：必须覆盖
 * - **组件测试**：引入 @vue/test-utils 后逐步补齐（MessageNode 等大组件拆分后）
 *
 * 运行：`npm test`（单次）/ `npm run test:watch`（监视模式）
 */
export default defineConfig({
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src')
    }
  },
  test: {
    // node 环境足够覆盖纯逻辑层；DOM 相关测试后续可用 // @vitest-environment happy-dom 标注
    environment: 'node',
    include: ['src/**/*.{test,spec}.ts'],
    // 被测代码与测试同目录（__tests__）或 *.spec.ts 命名
    globals: false
  }
})