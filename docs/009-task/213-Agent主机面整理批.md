---
kind: task
id: T-213-Agent主机面整理批
title: Agent 主机面整理批——桥接三组语义拆分、委托可观测、归属表机器校验
status: doing
created: 2026-10-20
track: plan
prio: later
deps: []
parent: T-183-质量与机制清偿总批
where: symbio/src/plugins/agent/host/subagent.rs · symbio/src/plugins/agent/host/mod.rs · symbio/src/plugins/agent/README.md · scripts/agent-ownership-audit.mjs · symbio/src/symbio_core/actors/commitment.rs · symbio/src/plugins/session/stats.rs
exit: '桥接函数的三组语义（审批与提问冒泡识别、消息过滤、节点改写）拆成三个可单测函数，改写顺序依赖（主体改写须在角色改写之后）保持且有判据；`agent_run` 的能力自述补可观测判据（父会话收束时若本轮发生过 `agent_run`，存在一条可查的委托记录）；主机模块文档里的归属表加机器校验（模块文档列的归属、装配清单、实际插件目录三者一致）；全量门禁与 e2e 全绿'
---

## 内容

父设计：09 报告建议 S-3 / S-6 / S-7

**三项都是「让 Agent 主机的内部结构可被引用」**：

| 项 | 现状 | 后果 |
|---|---|---|
| 桥接函数（238 行） | 三组语义挤在一个函数里 | 「审批冒泡的判据是什么」无法被引用、无法被单测 |
| 委托承诺 | 靠插件自述（`records_delegation_promise`） | 承诺不是事实，没人验证 |
| 归属表 | 手写、无人校验 | 文档里的模块与目录早就不一致（5 处过期描述另归文档批） |

**拆桥接函数的收益是「可测性」不是「行数」**（原报告特别强调）：三个纯函数之后，
三组语义各自有判据；238 行变成编排 + 三个有名字的函数，读的人能指着一段说「这里错了」。

**委托记录这条把「承诺」变「事实」**：判据是「父会话收束时，若本轮发生过 `agent_run`，
则存在一条可查的委托记录」——它不关心插件怎么声明，只关心事实有没有落下来。

**归属表机器校验**值得单独说：这类表（README 一处、模块文档一处、装配清单一处）失效的根因
是「手写 + 无人校验」。一条比对脚本比逐次人工核对便宜得多——这是本仓「约定 → 机制」的
标准动作，也是本批里唯一能一次消掉整类问题的项。

## 处理过程

- 2026-10-20 立批。来源：09 报告 S-3 / S-6 / S-7。
- 2026-10-20 与 [T-214](214-文档口径对账批二.md "T-214-文档口径对账批二") 的分工：
  agent README 的 5 处**过期内容**归文档批（一次性订正），本批做**校验机制**（防再漂）。
  两件事一起做才是完整的：只订正不加校验，下次还会漂；只加校验不订正，脚本一上来就红。
- 2026-10-20 排序：先 7（加校验，同时暴露存量不一致），再 4（按校验结果订正，在 T-214 里），
  最后 3（拆函数，纯重构）。
- 2026-10-21 段一（**S-7 机器校验 + 它当场暴露的存量漂移**）：
  新增 [scripts/agent-ownership-audit.mjs](../../scripts/agent-ownership-audit.mjs)（AO-001…006），
  三条来源对账：agent 侧文档（README + `host/*.rs` 的 `//!`）↔
  `symbio_core::ASSEMBLY_SUB_AGENT_PLUGINS` ↔ `plugins/` 实际目录 ↔ `host/` 实际模块文件。
  **只判形状、不判散文**：认 `东西|归谁|地址` 与 `模块|职责` 两张表、`- \`X\` 在其中` 这一条
  判词、`host/*.rs` 与 `../<插件>/README.md` 两个路径前缀——中文散文里提到 `` `work` ``
  不判，因为天天误报的守卫最后只会被豁免喂死（`grep-audit` S-012 的教训）。
  跑第一轮的输出就是 09 报告 D-4 数出的那 5 处（AO-003 两条：`work 在其中` /
  `vdfs 不在其中`；AO-005 三条：`host/instruction.rs`、`host/memory.rs`、`../work/README.md`），
  而**这一轮此前全绿**——`doc-link-audit` D-010 判反引号路径却放过了它们，
  所以 AO-005 是补的那一块，不是重复的一块。
  **归属表的三份副本没有合并**（README / `host/mod.rs` / `host/plugin.rs` 各一份）：
  Rust 的模块文档要就地可读，删副本等于让读代码的人去别处找边界；代价由 AO-002 付——
  一个「东西」在全部副本里只能有一个归谁与地址，改一处忘改另一处当场红。
  README 的实质订正不止那五条指路：本插件从「三件事在同一处」收成**两件**（托管 + 门槛），
  `AGENTS.md` 那一件按 [ADR-040](../010-adr/040.md) 指回 `memory`，
  落位表与闸门**不复述**（一个事实只有一个 owner，见 `symbio/src/plugins/memory/README.md`
  与规范附录 A.3）；
  「挂载根下 `AGENTS.md` 是保留名」那段随 `host/vdfs.rs` 的现行事实删掉（挂载点内没有任何保留名）。
  顺带清掉同一条事实的代码侧漂移三处（报告只数了 README，这些是同族）：
  `host/vdfs.rs` 与 `host/store.rs` 还在描述「工作区级 + 全局级双层」而
  `AgentDirScope` 只有 `Global` 一个变体；`host/detail.rs` 的「来源层级」表单里挂着
  一条**永远命不中**的 `workspace` 选项——纸面能力，删法与
  [T-201 段二](201-VDFS单源与物理层收紧批.md "T-201-VDFS单源与物理层收紧批") 同一口径。
  判据：`agent-ownership-audit.test.mjs` 10 条——基线最小仓库全绿（防空转）、六条码各一条
  注入变红、AO-004 双向各钉（列了没有的红 / 加了不登记也红）、豁免必须带理由（空理由仍红）、
  来源读不出必须 exit 2（不许退化成「没发现违规」）。
- 2026-10-21 T-214 的 S-4（agent README 5 处过期描述）**在本段一并落地**：
  校验与订正必须同批，否则脚本一上来就红（本批 2026-10-20 的分工条款已预见）。
  T-214 那边只剩「别处的文档口径」，不再重复订正这份 README。
- 2026-10-21 段二（**S-6 委托承诺 → 可查事实**）：承诺的**写侧**早就在（`note_delegation` →
  `take_derived` → `record_derived` 落 `commitment.*`），`turn_exec.test.rs::full_turn_lands_derived_commitment_facts`
  也钉过「一轮真委托 ⇒ 立约与违约都进了格」。缺的是**读侧**：`session/stats` 只有声誉列
  （把同一批格子折成一个分数），出口表 S08 那一行的「用户可见出口」是 `—`——
  于是「承诺违约率」这一格说得出目标数值，却指不出用户在哪里读它（T-040 登记的原始病根）。
  落法（三处，都不加新机制）：
  ① core 加读侧翻译 [`commitment_records`](../../symbio/src/symbio_core/actors/commitment.rs)
  （`symbio_core/actors/commitment.rs`）——与 `commitment_events` 成对，**写侧成格 / 读侧翻译
  各住一边**（同 `EliminationDecider::verdict_grants_changes`）：配对靠载荷 `id`，
  `asserted` 是同一条约的第三格不另计，找不到立约的了结**跳过**（fail-closed），
  输出按 `id` 字典序。以 `pub(crate)` 出根（`symbio_core/mod.rs`），记录类型随返回值流动、
  不在出口边界被命名——与 `ReputationView` 同形态，C-003 的单消费方棘轮一格没动（14/14）。
  ② `session/stats` 加 `commitments` 列（`symbio/src/plugins/session/stats.rs`）：
  `{ items, total, kept, broken, settled, breach_rate }`，归**四列那一族**（同切片同 as-of，
  `may_read = false` ⇒ `items: []` 的空形状而非缺列）。**违约率分母取已了结数**：
  在途的约既没守也没违约，把它算进分母等于把「这一轮还没跑完」读成「没做到」。
  面板加第五段「承诺」（有违约 ⇒ `Warn` 语气——S08 §5 的「违约必须可观测」落到界面上就是
  该标色的时候标）。
  ③ 出口表 `04 §1.3` 的 S08 行把 `—` 换成 `commitments`，`slo-exit-audit` 当场把它钉成
  「指得到一列读数」；无读数的行 10 → 9（这个数字每次门禁都打得出，是本段真正的出口）。
  判据四条新增（`cargo test -p symbio --lib` 1939 → 1941）：core 一条（三条口径各钉：
  `asserted` 不另计 / 在途是 `open` / 幽灵了结跳过 + 排序确定），stats 一条（一条违约 +
  一条在途 ⇒ 读得出 promise / to / why / anchor，率 = 1.0 而不是 0.5，无权读 ⇒ 空形状），
  面板同源一条 + 分区数一条。夹具调的是生产同一个成格口 `commitment_events`，改名两边一起红。
  **没有新增守卫脚本**：这一段的判据落在读数与列上（`slo-exit-audit` 已在守「指认必须落到
  真列」），再造一个脚本就是给同一条事实第二个判据源。
