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
     * ⚠️ **历史（教训）**：阈值曾长期停在 `40 / 40 / 38 / 38`，而 2026-09-20
     * 实测已 60.93%——`lines` 高出阈值 20 个点，**棘轮形同虚设**。当时未上调，
     * 理由写的是「本机 Windows / CI 跑 Linux，平台分支的覆盖可能不同，本地数字
     * 不足以代表 CI，上调前先取 CI 实测值」。该理由已于 2026-09-23 **证伪**：
     *
     *   grep -rE "process\.platform|skipIf|platform ===|process\.|require\(" tauri/src  ⇒ 0 命中
     *
     * 前端源码与测试**没有任何平台分支**（也没有 locale / 时区分支：`time.ts` 的
     * `toLocaleString()` 只被 `toMatch(/\d{4}/)` 断言，与 locale 无关）。而 CI 的
     * frontend 作业跑的就是**同一条** `vitest run --coverage`（`.github/workflows/ci.yml`）。
     * 既然分支与平台无关，「本地 vs CI 数字不同」这层差异**不存在**，等一次 CI
     * 实测值不会带来任何新信息——那个「先决条件」本身就是个不会兑现的门。
     *
     * 对照：棘轮的另一半（**用例数**）落在 `scripts/gate.d/_shared.mjs` 的 `BASELINE`，
     * 那一半早在 2026-09-20 就钉到了实测值（现为 49 文件 / 725 用例）。两半的病根
     * 相同（**没人拧的棘轮等于没有棘轮**），修法也应相同：**直接钉实测值**。
     *
     * 2026-09-23 实测（49 文件 / 725 用例，`npm run test:coverage`）：
     *   lines 67.72 / statements 66.53 / functions 64.12 / branches 62.61
     *   `src/registry/**`：lines 92.38 / statements 93.06
     * 下次上调：复测后同样「下取整 − 5」，并把新实测值写进各行的行尾注释。
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
        lines: 62, // 实测 67.72 → 下取整 67 − 5
        statements: 61, // 实测 66.53 → 66 − 5
        functions: 59, // 实测 64.12 → 64 − 5
        branches: 57, // 实测 62.61 → 62 − 5
        // 机制化核心单独设限（实测 92.38 / 93.06，同样 −5）
        'src/registry/**': { lines: 87, statements: 88 },
      },
    },
  }
})