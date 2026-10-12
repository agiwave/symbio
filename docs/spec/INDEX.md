# 规范登记索引（自动生成）

> ⚠️ **本文件由 `node scripts/registry-audit.mjs --write` 从 `docs/spec/*.md` 生成，请勿手改。**
> 状态的唯一登记处是各规范文件本身，本页是投影。字段规范见 [SP-001](./SP-001.md)，原则与流程见 [14 · 登记与汇总机制](../plan/PL-011-登记与汇总机制.md)。

**计数**：active 31 · draft 0 · retired 0（共 31）

## 现行规范（active）
| 编号 | 规范 | 适用范围 | 判据 | 清除 | 取代 |
|---|---|---|---|---|---|
| `SP-001` | 登记条目 front-matter 与状态规范 | 仓内全部活跃 `.md` 文档（九类 kind；豁免名单见 plan/14 §0） | 人工 | — | — |
| `SP-002` | ADR 的域分配——按「这条决策约束谁」归域，域是条目的可筛字段 | docs/adr/ 下全部决策条目的 domain 字段 | `scripts/registry-audit.mjs` | — | — |
| `SP-003` | PL-006 §0 纪律 1——先实现 → 单测 → e2e（含 LLM mock）→ 全门禁，绿才提交 | docs/plan/PL-006-分步实施计划.md §0 纪律 1（条文 owner 所在） | `scripts/gate.mjs` | — | — |
| `SP-004` | PL-006 §0 纪律 1b——跨批行为变更的中间态提交 | docs/plan/PL-006-分步实施计划.md §0.3（条文 owner 所在） | 人工 | — | — |
| `SP-005` | PL-006 §0 纪律 1c——原子批次：多开关必须同翻 | docs/plan/PL-006-分步实施计划.md §0.3b（条文 owner 所在） | 人工 | — | — |
| `SP-006` | PL-006 §0 纪律 2——每步一条新 e2e，且跑出厂默认路径 | docs/plan/PL-006-分步实施计划.md §0 纪律 2（条文 owner 所在） | 人工 | — | — |
| `SP-007` | PL-006 §0 纪律 3——架构改动必须逐条论证「必需」 | docs/plan/PL-006-分步实施计划.md §3（条文 owner 所在） | 人工 | — | — |
| `SP-008` | PL-006 §0 纪律 4——不做历史数据兼容 | docs/plan/PL-006-分步实施计划.md §0 纪律 4（条文 owner 所在） | 人工 | — | — |
| `SP-009` | PL-006 §0 纪律 5——完成判据必须含一条端到端判据（默认路径压过） | docs/plan/PL-006-分步实施计划.md §0.4（条文 owner 所在） | 人工 | — | — |
| `SP-010` | 纪律 1（13 §0）——只写目标态：注释只写不变量与陷阱，不写过程 | 全部活跃文档与源码注释（条文原属已归档的 plan/13 §0，随归档迁入本条目） | `scripts/grep-audit.mjs` | — | — |
| `SP-011` | 纪律 2（13 §0）——机制优先：每条问题落成守卫 / 真源 / 类型约束 | 全部改进批次与问题登记（条文原属已归档的 plan/13 §0，随归档迁入本条目） | 人工 | — | — |
| `SP-012` | 纪律 3（13 §0）——不新增真相：改文档前先确认那条事实的 owner | 全部文档改动（条文原属已归档的 plan/13 §0，随归档迁入本条目） | 人工 | — | — |
| `SP-013` | 纪律 4（13 §0）——出口可核对：判据要么是守卫，要么是脚本可核对的锚 | 全部任务的 exit 字段与批次出口（条文原属已归档的 plan/13 §0，随归档迁入本条目） | 人工 | — | — |
| `SP-014` | 纪律 5（13 §0）——豁免必有清除任务：无清除任务的豁免不许登记 | 守卫豁免注释 / 豁免账本条目（条文原属已归档的 plan/13 §0，随归档迁入本条目） | 人工 | — | — |
| `SP-015` | PL-004 §3.1 死码承认台账口径——承认挂计划步 + 棘轮只许降 + 终局基线 8 | docs/plan/PL-004-工程落地.md §3.1（22 步状态与死码清偿登记）；owner = scripts/dead-code-audit.mjs 的 R-002 段 | `scripts/dead-code-audit.mjs` | — | — |
| `SP-016` | PL-004 §1.3 逐阶出口表的口径——用户可见出口必须是 session/stats 真存在的一列 | docs/plan/PL-004-工程落地.md §1.3（逐阶出口 SLO 表）；owner = scripts/slo-exit-audit.mjs | `scripts/slo-exit-audit.mjs` | — | — |
| `SP-017` | PL-004 §4 CI 断言清单口径——每条断言必须落成一条可执行判据，禁止生产调用点 | docs/plan/PL-004-工程落地.md §4（CI 断言清单 C1–C29 / C-D2–C-D4）；owner = 各守卫与 verify 程序 | `scripts/no-direct-call-audit.mjs` | — | — |
| `SP-018` | plan/README §4 验证纪律 1——每条声称对应一条可执行断言（零断言的程序不可能失败） | docs/plan/README.md §4 验证纪律 1（条文 owner 所在） | `scripts/gate.d/_shared.mjs` | — | — |
| `SP-019` | plan/README §4 验证纪律 2——断言的输入不得包含结论 | docs/plan/README.md §4 验证纪律 2（条文 owner 所在） | 人工 | — | — |
| `SP-020` | plan/README §4 验证纪律 3——每条断言配一个反向用例（改输入结论必须变） | docs/plan/README.md §4 验证纪律 3（条文 owner 所在） | `scripts/gate.d/_shared.mjs` | — | — |
| `SP-021` | plan/README §4 验证纪律 4——「编译期强制」必须附已知编译不过的反例 | docs/plan/README.md §4 验证纪律 4（条文 owner 所在） | 人工 | — | — |
| `SP-022` | PL-003 §5.1 纪律 1——每条声称对应一条可执行断言 | docs/plan/PL-003-演进与验证.md §5.1 纪律 1（条文 owner 所在） | `scripts/gate.d/_shared.mjs` | — | — |
| `SP-023` | PL-003 §5.1 纪律 2——断言的输入不得包含结论 | docs/plan/PL-003-演进与验证.md §5.1 纪律 2（条文 owner 所在） | 人工 | — | — |
| `SP-024` | PL-003 §5.1 纪律 3——每条断言配一个反向用例 | docs/plan/PL-003-演进与验证.md §5.1 纪律 3（条文 owner 所在） | `scripts/gate.d/_shared.mjs` | — | — |
| `SP-025` | PL-003 §5.1 纪律 4——「编译期强制」必须附已知编译不过的反例 | docs/plan/PL-003-演进与验证.md §5.1 纪律 4（条文 owner 所在） | 人工 | — | — |
| `SP-026` | PL-003 §5.1 纪律 5——编译期反例必须写在外部模块里 | docs/plan/PL-003-演进与验证.md §5.1 纪律 5（条文 owner 所在） | 人工 | — | — |
| `SP-027` | C-003 根出口单消费方豁免账本（5 条） | scripts/core-export-audit.mjs 的 WAIVERS 账本（ActorSpec / turnstate / SloLatencyView / FallbackRateView / mutations_latest_snapshot） | `scripts/core-export-audit.mjs` | T-059 | — |
| `SP-028` | doc-link D-004 全文豁免（定义变更史禁区的两处反例措辞） | docs/archive/13-质量改进总纲.md 首行与 docs/README.md 首行的 doc-link-allow D-004 | `scripts/doc-link-audit.mjs` | T-060 | — |
| `SP-029` | dead-code R-002 契约登记存量（8 处） | scripts/dead-code-audit.mjs 的 R-002 死码承认账本（终局基线 8，见 docs/plan/PL-004-工程落地.md §3.1） | `scripts/dead-code-audit.mjs` | T-060 | — |
| `SP-030` | panic-allow 不变量登记（C7，56 个登记项 / 71 处 panic 面） | symbio/src/** 的 panic-allow 注释、scripts/panic-audit.mjs 账本与 scripts/_shared.mjs 的 BASELINE.panicSites | `scripts/panic-audit.mjs` | T-060 | — |
| `SP-031` | color 守卫自指豁免（测试与实现提到禁用模式） | scripts/color.test.mjs（守卫回归与实现里的禁用模式字样） | `scripts/color.test.mjs` | T-060 | — |

## 草案（draft）
（无）

## 废止（retired）
（无）
