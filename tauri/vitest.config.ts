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
    /**
     * 池类型：**必须显式写 `threads`**。
     *
     * 默认的 `forks`（子进程池）在本机 Windows + Node 22 上**跑完不退出**：
     * 测试全部通过、汇总行已打印，进程却挂在那里不结束——`timeout` 必被触发。
     * 现象极具误导性：本地 `npm test` 看起来"卡死"，CI 会一直等到超时，
     * 而报告里一切正常（这正是它难被发现的原因）。
     *
     * 实测同一批用例：`forks` 挂起 / `threads` 正常退出（`--no-file-parallelism`
     * 也挂，故与并发度无关，是池实现本身）。显式指定而非依赖默认值——默认值
     * 会随 vitest 版本变，而"挂不挂"不该由版本决定。
     */
    pool: 'threads',
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
     * - 全局阈值取**实测值向下取整再减 5**。那 5 点是给日常改动的余量
     *   （新加一个还没测的文件不至于立刻红），但已足以拦住「删掉一批测试」
     *   或「加一大坨没测的代码」这类**批量**回退——棘轮要拦的是批量，不是零碎；
     * - `src/registry/**` 单独设限：那是「前端零业务知识」的机制化核心
     *   （类型 → 呈现的映射表），它掉下去就是业务知识又溜回前端了。
     *
     * ⚠️ 阈值必须钉在实测值上：**没人拧的棘轮等于没有棘轮**。前端源码与测试
     * 没有平台 / locale / 时区分支，CI frontend 作业跑的是同一条 `vitest run --coverage`，
     * 所以「等一次 CI 实测值再上调」不会带来新信息。用例数那一半落在
     * `scripts/gate.d/_shared.mjs` 的 `BASELINE`，两半同一条纪律。
     * 上调口径：复测后同样「下取整 − 5」，并同步各行的行尾注释。
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
        lines: 67, // 实测 72.21 → 下取整 72 − 5
        statements: 61, // 实测 66.53 → 66 − 5
        functions: 59, // 实测 64.12 → 64 − 5
        branches: 57, // 实测 62.61 → 62 − 5
        // 机制化核心单独设限（实测 92.38 / 93.06，同样 −5）
        'src/registry/**': { lines: 87, statements: 88 },
      },
    },
  }
})