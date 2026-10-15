---
kind: task
id: T-080-adr-impl-档位逐条复核批
title: 「无人解释的 partial」清零：ADR-020/022/027/053 档位复核升档或列出残余；ADR-054 守卫退役条款裁定；ADR-027 备注补指认
status: done
created: 2026-10-13
track: 登记卫生
deps: []
prio: later
exit: 每条一个裁决，不许维持「partial 但正文不解释还差什么」——① [ADR-020](../010-adr/020.md)：其指认的模块文档（`symbio/src/plugins/session/docs/015-core-loop.md` §6/§7 相关行）与代码现实相反（流式分支已收敛），先修文档再复核档位；② [ADR-022](../010-adr/022.md)：残余仅剩「e2e 只走一条 SSE 形状」，该债已归 [T-076](076-e2e面缺失补齐批.md "T-076-e2e面缺失补齐批")——按残余归属关系决定升 done 或把残余写明在正文；③ [ADR-027](../010-adr/027.md)：备注补「部分取代，续接者见 [ADR-029](../010-adr/029.md)」指认（T-008 迁移记录载明）；④ [ADR-053](../010-adr/053.md)：b1/b2/b3 判据已由 T-053/T-054/T-055/T-056（done）承载，复核后升档或写明残余；⑤ [ADR-054](../010-adr/054.md)「守卫是过渡品该退役」与门禁现状（`scripts/gate.d/30-docs.mjs` 仍接 LWF-001…004，且注释口径停在 v1 棘轮）构成未裁定矛盾——裁定「转正为永久结构守卫（修订条款）」或登记退役批，并同步 `scripts/list-write-fact-audit.mjs` 注释
evidence: 五条裁决全部落为 `impl: done` + 正文残余说明（无一条留「partial 却不解释」）：① ADR-020——订正 `symbio/src/plugins/session/docs/015-core-loop.md` §6.4 那句与 §7/§8 相反的「工具经自有 `PluginPayload::Session` 通道回传事件…尚未收敛」（实测 `PluginPayload::Session` 在 `session/**` 只剩注释指认，`execute_tool_async` 明写「不再有流式分支」），ADR-020 加复核补记升 done；② ADR-022——六条决策全落地（`parse_line` / `open_partial_line` / 边界对齐 / 三钩子全在 `plugins/model/protocols/`），残余只在**覆盖面**（e2e 只走 `openai_chat` 一条形状）且已归 T-076，属测试矩阵的债非本条落地缺口 ⇒ 升 done；③ ADR-027——正文补「部分取代」两行表（导入入口/前端选入口 → ADR-029；`path` 归属 → ADR-030，两者 `impl: done`），并说明为何不写 `supersedes`（那个字段只表达整体取代）⇒ 升 done；④ ADR-053——三批落地锚 `e62d8a85`（b1）/`fd844424`（b2）/`941cf88d`+`e4743c11`（b3）与承载任务 T-053…T-056 成表 ⇒ 升 done；⑤ ADR-054——裁定「过渡形态退役、结构守卫**转正**为永久守卫」（守的是决策 4 那条持续不变量「没有任何地方能改列表而不落事实」，类型只管得住 `apply` 一侧），决策 3 末句按修订读、实施补记 4 记录；`scripts/gate.d/30-docs.mjs` 接线注释由 v1（就地登记 + pending 棘轮 + LWF-001…004）改写为 v2（结构判定 + LWF-001–002 + 寄存锚口径）。顺带把复核确立的规则写回 [SP-001](../008-spec/001.md) 的 `impl` 字段行（「档位必须可解释」）。验证：`node scripts/gen-doc-refs.mjs` 展开 11 处零 RF-003 · `node scripts/registry-audit.mjs` 零违规 · `gate --only=docs,facts` 绿
---

## 内容

本轮 ADR 全量复核（2026-10-13）确立的规则：**`impl` 档位必须可解释**——done 有判据、
partial 的每处残余有任务归属。ADR-020/022/027/053/054 五处属于「档位与代码/任务现状不符
或未裁定」，逐条收口；同批清掉两处陈旧注释（AR-015、30-docs.mjs）。这是登记机制（RG 判据面）
补不到的语义层，靠本任务一次性清偿；清偿后新增 partial 必须自带残余归属，由复核顺带确立。

## 处理过程

- 2026-10-13 登记（ADR 全量复核报告：无任务承载的残余清单第 4 项）。
- 2026-10-13 清偿（五条裁决一次做完，不留半条）：
  - **① ADR-020**：先核代码——`grep -rn "PluginPayload::Session" symbio/src/plugins/session/`
    只回 `tools/tool_executor.rs` 的两行**注释**，且 `execute_tool_async` 的文档明写
    「本函数**不再有流式分支**」。故 AR-015 §6.4 那句「工具经自有通道回传事件…尚未收敛」
    是**陈旧**的，改写成「已收敛到 `EventSink`，`PluginPayload::Session` 只剩跨进程用途」。
    随后 ADR-020 六条决策逐条对账（§6.1–§6.3 表 + 代码），加复核补记升 `done`；
    决策 7 被 ADR-021 推翻，明写**不属本条残余**（避免被当成 partial 的残留项）。
  - **② ADR-022**：判定依据是「残余属**测试覆盖面**、不属决策落地」——四个协议的
    `parse_line` / `open_partial_line` 都在 `plugins/model/protocols/` 里，core 不再认识字段名；
    e2e 只走 `openai_chat` 一条形状的债归 T-076。升 `done` 并把这层区分写进正文
    （否则后人会把「覆盖债」重新算回本条 partial）。
  - **③ ADR-027**：从 `git show 50acd3dd^:docs/DECISIONS.md` 取回迁移前的权威口径
    （原文是「**部分现行**——『至多一个』仍成立；导入入口/前端选入口被 ADR-029 取代、
    `path` 归属被 ADR-030 取代」）。据此把「部分取代」两行表写进正文（补上任务 exit
    未提的 ADR-030），并说明**为何不写 `supersedes`**：`active` 禁 `reason`、登记机制
    无「部分取代」状态词，而 `supersedes` 只表达整体取代，挂上去会把「至多一种仍现行」
    谎报成作废。`impl: partial → done`（存活的决策已落地，取代方两条均已闭环）。
  - **④ ADR-053**：定位三批落地锚（`git log --grep=S2.7`）：b1 = `e62d8a85`、
    b2 = `fd844424`、b3 = `941cf88d` + `e4743c11`；承载任务 T-053/T-054/T-055/T-056
    全 `done`；事件名在 `symbio_core::event` 有常量、PL-001 §6 有 `message` 行。升 `done`。
  - **⑤ ADR-054**：裁定**转正**而非退役。理由：决策 4 的出口判据
    「没有任何地方能改列表而不落事实」是**持续不变量**，类型强制只管 `apply` 一侧，
    「有没有第二条路」是结构性质——只有机械判定能持续保证（正是本条理由 3
    「人工审计适合发现、不适合保证」的同一逻辑）。决策 3 末句改为按修订读，
    实施补记 4 记裁定与配套；`30-docs.mjs` 接线注释 v1→v2（含 LWF-003/004 退役说明）。
  - **规则沉淀**：本轮复核确立的「`impl` 档位必须可解释」写回 SP-001 的 `impl` 字段行
    ——否则清偿成果只留在任务里，下一条 partial 又会以「没人解释」的形态出现。
  - **验证**：`gen-doc-refs` 展开 11 处（`[[编号]]` 记号，零 RF-003）；`registry-audit`
    零违规；`gate --only=docs,facts` 绿。
