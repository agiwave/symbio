/**
 * 命名档位：CI job 与 git 钩子只引用「档位名」，不各自抄一份 `--only=` 阶段清单。
 *
 * ## 为什么要有这一层
 *
 * 阶段 id 的组合此前分散在 7 处（`ci.yml` 5 个 job、`pre-commit`、`pre-push`）。
 * 阶段改名或拆出时，漏改一处**不报错**：`gate.mjs` 只拦「不存在的 id」，
 * 对「存在但少点了一个阶段」什么都不说，于是那一路悄悄少跑一个 job、日志全绿。
 * `pre-commit` 就断过一次（`--only=facts` 少了 `facts-write`，汇总页投影落后两提交）。
 *
 * 档位是**单点**：组合只在这里写一次；调用方漏写档位名会当场报「未知档位」，
 * 而不会退化成一次别的动作。
 *
 * ## 边界
 *
 * 档位只决定「跑哪些阶段」，不决定模式：`--ci` 仍由调用点显式传（CI / `pre-push`
 * 传，本地生成链不传——它要改写并暂存）。这条边界由 `_shared.test.mjs` 的
 * 「点了 facts 复核就必须点 facts-write」与「facts 调用必须带 --ci（CI 侧）」两条
 * 判据守着，档位化不改变那些判定。
 *
 * 加一个档位 = 在这里加一行；`gate.mjs --list` 会把它连阶段一起列出来。
 */
// @gate lib

export const PRESETS = Object.freeze({
  /** 提交点：生成链 + 末态复核（本地模式会暂存生成器自己改写的文件）。 */
  'pre-commit': ['facts-write', 'facts'],
  /** 推送点：格式化 + 生成链 + 末态复核（`--ci`：只报红不代改）。 */
  'pre-push': ['fmt', 'facts-write', 'facts'],
  /** Rust 构建与测试（CI 的 rust-checks job；`fmt` 不能省，见 05-fmt 文件头）。 */
  rust: ['fmt', 'backend'],
  /** 最低支持 Rust 版本（CI 的 msrv-check job）。 */
  msrv: ['msrv'],
  /** 验证程序（CI 的 verify-evidence job）。 */
  verify: ['verify'],
  /** 前端静态检查 + 构建（vite build 必须留在批后，见 56-frontend-build）。 */
  frontend: ['frontend-static', 'frontend-build'],
  /** 端到端用例（CI 的 e2e job）。 */
  e2e: ['e2e'],
  /** 文档 + 事实 + 基线（CI 的 docs & facts job）。 */
  'docs-facts': ['facts-write', 'docs', 'facts', 'baseline', 'baseline-scope'],
  /** 周档：`@gate weekly` 桶（`weekly.yml` 的 schedule job；docs-facts 档位不含它，D-496）。 */
  weekly: ['weekly'],
})

/** 档位名 → 阶段 id 数组；未知档位返回 null（调用方负责拒绝，不静默退化成全跑）。 */
export function presetIds(name) {
  return Object.prototype.hasOwnProperty.call(PRESETS, name) ? [...PRESETS[name]] : null
}
