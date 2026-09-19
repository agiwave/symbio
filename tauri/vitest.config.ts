import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'
import path from 'path'
import { fileURLToPath } from 'url'

// vitest v4 native config loader 不支持 CommonJS 的 __dirname，
// 统一从 import.meta.url 派生（ESM 环境下 node 与 vitest 均可用）
const __dirname = path.dirname(fileURLToPath(import.meta.url))

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
    globals: false,

    /**
     * 覆盖率：**棘轮**，只升不降。
     *
     * 阈值的意义不是「追求某个百分比」，而是让覆盖率**不能再降**——
     * 加代码不带测试、或删掉一批测试，门禁会红。
     *
     * 两条口径：
     * - 全局阈值取实测值向下取整（实测 42.27% 行覆盖 → 阈值 40，留一点余量，
     *   免得日常改动动不动就红而被人绕过）；
     * - `src/registry/**` 单独设限：那是「前端零业务知识」的机制化核心
     *   （类型 → 呈现的映射表），它掉下去就是业务知识又溜回前端了。
     *
     * 有提升就上调这里（改完跑一次 `npm run test:coverage` 确认绿）。
     */
    coverage: {
      provider: 'v8',
      include: ['src/**/*.{ts,vue}'],
      exclude: [
        'src/**/__tests__/**',
        'src/**/*.spec.ts',
        'src/main.ts', // 应用入口：只做挂载，无逻辑可测
        'src/types/**', // 纯类型声明
      ],
      // 只看文本报告：html/json 产物会在工作区留下垃圾且无人读
      reporter: ['text'],
      thresholds: {
        lines: 40,
        statements: 40,
        functions: 38,
        branches: 38,
        // 机制化核心单独设限（实测 88%，留 8 点余量）
        'src/registry/**': { lines: 80, statements: 80 },
      },
    },
  }
})