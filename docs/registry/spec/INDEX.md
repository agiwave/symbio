# 规范登记索引（自动生成）

> ⚠️ **本文件由 `node scripts/registry-audit.mjs --write` 从 `docs/registry/spec/*.md` 生成，请勿手改。**
> 状态的唯一登记处是各规范文件本身，本页是投影。字段规范见 [SP-001](./SP-001.md)，原则与流程见 [14 · 登记与汇总机制](../../plan/14-登记与汇总机制.md)。

**计数**：active 31 · draft 0 · retired 0（共 31）

## 现行规范（active）
| 编号 | 规范 | 适用范围 | 判据 | 清除 | 取代 |
|---|---|---|---|---|---|
| `ADR-DOM` | ADR 的域分配——按「这条决策约束谁」归域，域是条目的可筛字段 | docs/registry/adr/ 下全部决策条目的 domain 字段 | `scripts/registry-audit.mjs` | — | — |
| `D08-1` | 08 §0 纪律 1——先实现 → 单测 → e2e（含 LLM mock）→ 全门禁，绿才提交 | docs/plan/08-分步实施计划.md §0 纪律 1（条文 owner 所在） | `scripts/gate.mjs` | — | — |
| `D08-1b` | 08 §0 纪律 1b——跨批行为变更的中间态提交 | docs/plan/08-分步实施计划.md §0.3（条文 owner 所在） | 人工 | — | — |
| `D08-1c` | 08 §0 纪律 1c——原子批次：多开关必须同翻 | docs/plan/08-分步实施计划.md §0.3b（条文 owner 所在） | 人工 | — | — |
| `D08-2` | 08 §0 纪律 2——每步一条新 e2e，且跑出厂默认路径 | docs/plan/08-分步实施计划.md §0 纪律 2（条文 owner 所在） | 人工 | — | — |
| `D08-3` | 08 §0 纪律 3——架构改动必须逐条论证「必需」 | docs/plan/08-分步实施计划.md §3（条文 owner 所在） | 人工 | — | — |
| `D08-4` | 08 §0 纪律 4——不做历史数据兼容 | docs/plan/08-分步实施计划.md §0 纪律 4（条文 owner 所在） | 人工 | — | — |
| `D08-5` | 08 §0 纪律 5——完成判据必须含一条端到端判据（默认路径压过） | docs/plan/08-分步实施计划.md §0.4（条文 owner 所在） | 人工 | — | — |
| `D13-1` | 纪律 1（13 §0）——只写目标态：注释只写不变量与陷阱，不写过程 | 全部活跃文档与源码注释（条文原属已归档的 plan/13 §0，随归档迁入本条目） | `scripts/grep-audit.mjs` | — | — |
| `D13-2` | 纪律 2（13 §0）——机制优先：每条问题落成守卫 / 真源 / 类型约束 | 全部改进批次与问题登记（条文原属已归档的 plan/13 §0，随归档迁入本条目） | 人工 | — | — |
| `D13-3` | 纪律 3（13 §0）——不新增真相：改文档前先确认那条事实的 owner | 全部文档改动（条文原属已归档的 plan/13 §0，随归档迁入本条目） | 人工 | — | — |
| `D13-4` | 纪律 4（13 §0）——出口可核对：判据要么是守卫，要么是脚本可核对的锚 | 全部任务的 exit 字段与批次出口（条文原属已归档的 plan/13 §0，随归档迁入本条目） | 人工 | — | — |
| `D13-5` | 纪律 5（13 §0）——豁免必有清除任务：无清除任务的豁免不许登记 | 守卫豁免注释 / 豁免账本条目（条文原属已归档的 plan/13 §0，随归档迁入本条目） | 人工 | — | — |
| `S31-1` | 04 §3.1 死码承认台账口径——承认挂计划步 + 棘轮只许降 + 终局基线 8 | docs/plan/04-工程落地.md §3.1（22 步状态与死码清偿登记）；owner = scripts/dead-code-audit.mjs 的 R-002 段 | `scripts/dead-code-audit.mjs` | — | — |
| `S31-2` | 04 §1.3 逐阶出口表的口径——用户可见出口必须是 session/stats 真存在的一列 | docs/plan/04-工程落地.md §1.3（逐阶出口 SLO 表）；owner = scripts/slo-exit-audit.mjs | `scripts/slo-exit-audit.mjs` | — | — |
| `S31-3` | 04 §4 CI 断言清单口径——每条断言必须落成一条可执行判据，禁止生产调用点 | docs/plan/04-工程落地.md §4（CI 断言清单 C1–C29 / C-D2–C-D4）；owner = 各守卫与 verify 程序 | `scripts/no-direct-call-audit.mjs` | — | — |
| `SP-001` | 登记条目 front-matter 与状态规范 | docs/registry/ 下全部登记条目（task / spec / adr 及后续新增的 kind） | 人工 | — | — |
| `V4-1` | plan/README §4 验证纪律 1——每条声称对应一条可执行断言（零断言的程序不可能失败） | docs/plan/README.md §4 验证纪律 1（条文 owner 所在） | `scripts/gate.d/_shared.mjs` | — | — |
| `V4-2` | plan/README §4 验证纪律 2——断言的输入不得包含结论 | docs/plan/README.md §4 验证纪律 2（条文 owner 所在） | 人工 | — | — |
| `V4-3` | plan/README §4 验证纪律 3——每条断言配一个反向用例（改输入结论必须变） | docs/plan/README.md §4 验证纪律 3（条文 owner 所在） | `scripts/gate.d/_shared.mjs` | — | — |
| `V4-4` | plan/README §4 验证纪律 4——「编译期强制」必须附已知编译不过的反例 | docs/plan/README.md §4 验证纪律 4（条文 owner 所在） | 人工 | — | — |
| `V51-1` | 03 §5.1 纪律 1——每条声称对应一条可执行断言 | docs/plan/03-演进与验证.md §5.1 纪律 1（条文 owner 所在） | `scripts/gate.d/_shared.mjs` | — | — |
| `V51-2` | 03 §5.1 纪律 2——断言的输入不得包含结论 | docs/plan/03-演进与验证.md §5.1 纪律 2（条文 owner 所在） | 人工 | — | — |
| `V51-3` | 03 §5.1 纪律 3——每条断言配一个反向用例 | docs/plan/03-演进与验证.md §5.1 纪律 3（条文 owner 所在） | `scripts/gate.d/_shared.mjs` | — | — |
| `V51-4` | 03 §5.1 纪律 4——「编译期强制」必须附已知编译不过的反例 | docs/plan/03-演进与验证.md §5.1 纪律 4（条文 owner 所在） | 人工 | — | — |
| `V51-5` | 03 §5.1 纪律 5——编译期反例必须写在外部模块里 | docs/plan/03-演进与验证.md §5.1 纪律 5（条文 owner 所在） | 人工 | — | — |
| `WVR-01` | C-003 根出口单消费方豁免账本（5 条） | scripts/core-export-audit.mjs 的 WAIVERS 账本（ActorSpec / turnstate / SloLatencyView / FallbackRateView / mutations_latest_snapshot） | `scripts/core-export-audit.mjs` | W1 | — |
| `WVR-02` | doc-link D-004 全文豁免（定义变更史禁区的两处反例措辞） | docs/archive/13-质量改进总纲.md 首行与 docs/README.md 首行的 doc-link-allow D-004 | `scripts/doc-link-audit.mjs` | W2 | — |
| `WVR-03` | dead-code R-002 契约登记存量（8 处） | scripts/dead-code-audit.mjs 的 R-002 死码承认账本（终局基线 8，见 docs/plan/04-工程落地.md §3.1） | `scripts/dead-code-audit.mjs` | W2 | — |
| `WVR-04` | panic-allow 不变量登记（C7，56 个登记项 / 71 处 panic 面） | symbio/src/** 的 panic-allow 注释、scripts/panic-audit.mjs 账本与 scripts/_shared.mjs 的 BASELINE.panicSites | `scripts/panic-audit.mjs` | W2 | — |
| `WVR-05` | color 守卫自指豁免（测试与实现提到禁用模式） | scripts/color.test.mjs（守卫回归与实现里的禁用模式字样） | `scripts/color.test.mjs` | W2 | — |

## 草案（draft）
（无）

## 废止（retired）
（无）
