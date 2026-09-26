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
// 不是**判定型**审计脚本，只跑回归测试（共享库 / 门禁原语 / 报告型脚本）：
//   - `color` 带一道「scripts/ 下不得手写 ANSI」守卫；
//   - `gate.d/_shared` 的 `autoWork` 是「自动执行的工作」原语。它的失效方式
//     与守卫同源且更隐蔽：**看起来在修、其实没把修复带进提交**——本地跑一次门禁
//     完全看不出来（文件确实被格式化了），只在「修复前就已脏/已暂存」时暴露。
//   - `cli-binary` 是「二进制必须对应当前源码」的原语。它的失效方式是**跑起来了
//     但跑的是过期产物**：用例照常执行、断言照常失败，只是失败形态与眼前的源码
//     矛盾（源码里明明有的字段，运行时是 undefined），把排查方向引到源码上。
//     回归测试钉住「指纹必须随构建输入变」——判据退化成「文件存在就算新鲜」时会红。
//   - `tauri-binary` 是同一个原语的**壳侧**那一份，失效方式更贵：它不回显错误、
//     只是让**日志**看起来来自当前源码。于是「日志里有一条源码中不存在的行」会被
//     当成「代码没接上」去读一遍代码，而真相是跑着过期产物。回归测试钉住三件事：
//     指纹随输入变（含 `symbio/src`——整棵插件树被编译进壳）、
//     「产物比构建戳旧」必须判不可信（构建失败时旧产物会看起来新鲜）、
//     输入未变时不得重写戳（否则「什么都不用重建」会变成假警报）。
//   - `schema-audit` 是**报告型**（见下 `REPORT_ONLY`），失效形态不是假绿灯而是
//     **说假话**：把在用的模块列成「下放候选」，读的人顺着去改本来没坏的东西。
//     实测事故：`schemas/hook` 被报成「仅 1 个外部消费文件」，而它实际有 3 个消费文件
//     ——hook 插件走 `schemas::{HookEvent}` 顶层再导出名，路径里没有子模块名。
//     报告不判失败 ⇒ 坏了没人发现，故它比判定型守卫**更需要**回归测试。
//   - `core-surface-audit` 同样是报告型，同样说假话的形态：**少算消费方** ⇒ 在用的
//     符号被列成「下放候选」。开发时真踩了两次，两条都写成了回归测试：① 三条
//     `pub use <域>::*` 撞进同一个 Map 键 ⇒ 公开面从 251 掉到 134，`PLUGIN_*` /
//     `PathKey` / `PluginStopReason` 全部消失；② `symbio/src` 直属文件（`lib.rs` /
//     `plugins/mod.rs`）被跳过 ⇒ 只在注册表里被用到的符号被算成 **0 个消费方**。
//     「数不到」与「真的没人用」是两件事。
const TEST_ONLY = [
  'color',
  'gate.d/_shared',
  'cli-binary',
  'tauri-binary',
  'schema-audit',
  'core-surface-audit',
]
// 报告型：只防崩溃（退出码恒 0，判定需人工复核），走日志不刷屏。
// 刻意 `echo: 'none'`：这份报告的候选会长期存在（大部分是签名组成部分与自引用），
// 每次门禁都刷一遍只会训练人忽略它。要看结论就单独跑一次脚本。
const REPORT_ONLY = ['schema-audit', 'core-surface-audit']

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
