// fmt 屏障阶段：格式化是**门禁自己做的事**（见 `_shared.autoWork`），但它同时是
// 阶段级并发的**屏障**——backend 的编译与 docs 守卫（panic-audit / dead-code-audit /
// doc-symbol-audit 等）都逐文件读 `.rs`，而 rustfmt 就地重写文件**不是原子操作**，
// 与任何读方并发都是「读到半截文件 ⇒ 假红」。所以 fmt 单独成阶段、排在所有
// `parallel: true` 泳道之前跑完，之后各泳道看到的源码就是定格的。
//
// ⚠️ CI 的 rust-checks job 因此必须是 `--only=fmt,backend`（不能只 `--only=backend`）：
//    少了 fmt，CI 就没有「格式化差异必须判红」这道闸——autoWork 的 CI 语义是
//    「跑完仍有差异 ⇒ 红」，它随本阶段一起从 backend 的 `--only` 清单里被划走了。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import { autoWork } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')

export default {
  id: 'fmt',
  title: '格式化（屏障：先于所有并发泳道）',
  tasks(ctx) {
    if (!fs.existsSync(path.join(repoRoot, 'Cargo.toml'))) return []
    return [
      {
        label: 'cargo fmt --all（自动格式化）',
        run: (c) => autoWork(c, { label: 'cargo fmt --all', cmd: 'cargo', args: ['fmt', '--all'], cwd: repoRoot }),
      },
    ]
  },
}
