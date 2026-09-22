// docs 阶段：静态审计守卫。
// 判定型守卫的**回归测试**先跑：一个只会亮绿灯的守卫等于没有守卫——它腐烂的
// 方式恰恰是「规则写错了所以永远不命中」，只有注入真实违规并断言脚本变红，
// 才能区分「通过」与「没在工作」。
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')

// 带回归测试的判定型守卫（两份清单保持同序同集）
const GUARDS = [
  'grep-audit',
  'mechanism-audit',
  'plugin-entry-audit',
  'protocol-mirror-audit',
  'style-audit',
  'doc-link-audit',
  'test-layout-audit',
  'dead-code-audit',
]
// `color` 不是审计脚本而是共享库，但带一道「scripts/ 下不得手写 ANSI」守卫，只跑回归测试。
const TEST_ONLY = ['color']
// 报告型：只防崩溃（退出码恒 0，判定需人工复核），走日志不刷屏。
const REPORT_ONLY = ['schema-audit']

export default {
  id: 'docs',
  title: '静态审计',
  *tasks() {
    for (const name of [...GUARDS, ...TEST_ONLY]) {
      yield {
        label: `${name} 回归测试`,
        cmd: process.execPath,
        args: ['--test', path.join(scriptDir, '..', `${name}.test.mjs`)],
        cwd: repoRoot,
      }
    }
    for (const name of GUARDS) {
      yield {
        label: `scripts/${name}.mjs`,
        cmd: process.execPath,
        args: [path.join(scriptDir, '..', `${name}.mjs`)],
        cwd: repoRoot,
        echo: 'all',
      }
    }
    for (const name of REPORT_ONLY) {
      yield {
        label: `scripts/${name}.mjs（报告型）`,
        cmd: process.execPath,
        args: [path.join(scriptDir, '..', `${name}.mjs`)],
        cwd: repoRoot,
        echo: 'none',
      }
    }
  },
}
