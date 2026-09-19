/**
 * ESLint 配置 —— 把「分层」从正则守卫升级成真正的静态检查
 *
 * ## 为什么要有它
 *
 * `scripts/mechanism-audit.mjs` 的 M-003 / M-004 / M-005 是**文本匹配**：
 * 它看不见重导出、看不见 `import type`、也看不见动态导入。三条规则真正想守住
 * 的东西——「出站请求只走 services」「契约层零呈现依赖」「service 不认识
 * store」——本质是**依赖方向**，用 import 图表达才准确。
 *
 * 所以这里只做一件事：**用 `no-restricted-imports` 把依赖方向钉死**。
 * 其余风格问题交给 `vue-tsc` 与既有审计脚本，不重复造门禁。
 *
 * ## 与 mechanism-audit 的分工
 *
 * 两者**并存**，不互相替代：
 * - 本文件管**能不能 import**（结构化、可靠、可 IDE 实时提示）；
 * - mechanism-audit 管**文件里写了什么**（字面量地址、词表比较、meta 字段解释）。
 *   那是文本层面的业务知识泄漏，import 图看不见。
 *
 * 跑法：`npx eslint .`（或 `npm run lint`）；CI 经 `scripts/gate.mjs` 的 frontend 阶段。
 */

import tseslint from 'typescript-eslint'
import pluginVue from 'eslint-plugin-vue'

/** 前端跑在 WebView 里：这些是真全局，不是拼写错误 */
const BROWSER_GLOBALS = {
  window: 'readonly',
  document: 'readonly',
  localStorage: 'readonly',
  navigator: 'readonly',
  console: 'readonly',
  AudioContext: 'readonly',
  setTimeout: 'readonly',
  clearTimeout: 'readonly',
  setInterval: 'readonly',
  clearInterval: 'readonly',
  requestAnimationFrame: 'readonly',
}

/** 依赖方向的唯一真相表：改分层就改这里，不要在各处另写 */
const LAYERING = [
  {
    // M-005：契约层零呈现 / 零状态依赖（防环：schemas 是最底层）
    files: ['src/schemas/**/*.ts'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            { group: ['@/registry', '@/registry/*'], message: '契约层不得依赖呈现层' },
            { group: ['@/components', '@/components/*'], message: '契约层不得依赖组件' },
            { group: ['@/composables', '@/composables/*'], message: '契约层不得依赖组合式' },
            { group: ['@/stores', '@/stores/*'], message: '契约层不得依赖 store' },
            { group: ['@/services', '@/services/*'], message: '契约层不得依赖服务层' },
          ],
        },
      ],
    },
  },
  {
    // M-004：映射层不得 import 组件（*Renderers.ts 是**刻意的**装配点）
    files: ['src/registry/**/*.ts'],
    ignores: ['src/registry/*Renderers.ts'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              group: ['**/*.vue'],
              message: '映射层不得 import 组件 —— 装配只在 *Renderers.ts 一处',
            },
          ],
        },
      ],
    },
  },
  {
    // service 不认识 store：状态是调用方的事，service 只管收发与纯逻辑。
    // （2026-09-19 修掉的两处反向依赖就是被这条钉住的：落地目标改为注入。）
    files: ['src/services/**/*.ts'],
    // 测试例外：`services/__tests__/*.spec.ts` 要**造一个真实 store 注入**给被测
    // service（依赖倒置正是为了让这种注入成为可能）。规则约束的是生产代码的
    // 依赖方向，测试里的 import 不构成那条依赖——它只是把它接上。
    ignores: ['src/services/__tests__/**'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              group: ['@/stores', '@/stores/*'],
              message: 'service 不得依赖 store —— 落地目标由调用方注入（依赖倒置）',
            },
          ],
        },
      ],
    },
  },
  {
    // M-003：出站请求只走 services（services 自身除外——它就是那一层）
    files: ['src/**/*.{ts,vue}'],
    ignores: ['src/services/**/*.ts'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          paths: [
            {
              name: '@tauri-apps/api/core',
              message: '不得直接 invoke —— 一律经 services/（协议切换只改那一层）',
            },
          ],
        },
      ],
    },
  },
]

export default tseslint.config(
  { ignores: ['dist/**', 'node_modules/**', 'src-tauri/**'] },
  ...pluginVue.configs['flat/essential'],
  // `.ts` 必须显式换回 TS 解析器：vue 的解析器只认 SFC，用它解析 .ts 会满屏
  // "Unexpected token"（`interface` / `enum` / 类型注解全炸）。
  {
    files: ['src/**/*.ts'],
    languageOptions: { parser: tseslint.parser, globals: BROWSER_GLOBALS },
  },
  {
    files: ['src/**/*.vue'],
    languageOptions: {
      // SFC 的 `<script lang="ts">` 要经 vue 解析器**再**转交 TS 解析器
      parserOptions: { parser: tseslint.parser },
      globals: BROWSER_GLOBALS,
    },
    rules: {
      // 组件内改 prop 是 Vue 的分层违规：props 是单向数据流，改了父组件不知情。
      // 它与「service 反向依赖 store」是同一类问题——绕过了拥有者。
      'vue/no-mutating-props': 'error',
      // 关掉「组件名必须多词」：本项目的 `Session.vue` / `About.vue` 是领域名而非
      // HTML 元素名，改名为 `TheSession` 只为满足一条防碰撞启发式，不划算。
      // （真与 HTML 元素冲突时 vue/essential 的其它规则会报。）
      'vue/multi-word-component-names': 'off',
      // 其余风格规则一律不加：风格交给 vue-tsc 与既有审计，
      // 一个报三百条风格告警的 lint 最后一定被 `--no-verify` 绕过。
    },
  },
  ...LAYERING,
)
