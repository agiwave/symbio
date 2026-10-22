---
kind: task
id: T-241-ActorDecl执行体注册表
title: ActorDecl 执行体注册表——by 名字到执行体一个动作，派发器业务分支退役
status: done
created: 2026-10-22
track: plan
parent: T-236-义务环符合性收口批
prio: next
deps: [T-238-短路轮进债环]
where: symbio/src/symbio_core/decl/ · symbio/src/plugins/loop/turn_exec.rs · symbio/src/plugins/loop/port.rs · symbio/src/plugins/loop/loop_rows.rs
exit: 'PL-016 §2.3 / §3.2 的**声明面**落地（形状按 [ADR-119](../010-adr/119.md) 裁决）：`ActorDecl { name, mode, reads, writes }` 住机制层、`run` 不是它的字段——可调用面与声明同批注册在部署层（`turn_exec::ACTORS` 的 `ActorEntry { decl, run }`），「能选中 ⇔ 有人跑」由 `Gap::UnmannedRow` / `Gap::OrphanActor` 双向装载检查保证；`by: Some(name)` 必须能在注册表解析到执行体，**`decl::ring::drive` 的 `accept` 谓词退役、签名退回 `(rows, frozen, events, now)`**（ADR-118 订正 1），未接线的执行体行显式改 `by = None` 占位、接线时与 `ActorDecl` 注册同批改回 `Some`；`turn_exec` 的 `Executor::for_by` 名字 match 与「模型 vs 工具批次」if/else 退役——引擎对 `by` 只有一个动作（`Next` 带动作本体，派发点是一次函数调用）；装载期新增第 6 条的两半（`ActorCannotClose` / `ActorModeTooNarrow`）；守卫用例 `accept_skips_rows_without_an_executor` 作废，代之以「`by=Some` 无注册 ⇒ 装载红」用例；单次提交结构性改动 ≤ 300 行；全量门禁与 e2e 绿。**`run` 的值返回形状（`Result<Vec<Fact>, ActorError>`）与「`Err` ⇒ 引擎写 `failed`」不在本任务**——拆给 [T-249-执行体值返回账本](249-执行体值返回账本.md)（理由与分界见 ADR-119 后果节）。'
evidence: '提交 `7a26d086` 之后的 v2 分支单批。① **声明面**：`decl/mod.rs` 新增 `ActorDecl { name, mode: LatencyTier, reads, writes }`（`Copy`，四样全静态可查）+ `Builder::actor` + `Frozen::actor/actors`；`seal()` 多两道错（`DuplicateActor` / `UnknownReadView`，后者对 `reads` 逐名查视图存在）。`builtin()` 改成 `builtin_with(&[ActorDecl])`——声明表与执行体注册表一起冻结，旧入口无残留调用方（`mod.test.rs` / `port.test.rs` 共 11 处改 `builtin_with(&[])`）。② **`accept` 退役**：`ring::drive` 签名退回 `(rows, frozen, events, now)`，可还性只剩 §2.2 的 `blocked`（`by = None ∨ unless`）；PTS-003 求解器指纹同批上调 `45977555…`→`b4aa7ddd…`，`adr: "ADR-118 ADR-119"` + 理由写明「机制演进、`owed` 字节未变」。③ **装载检查第 6 条补齐**：`check_rows` 新增 `UnmannedRow`（`by=Some` 查不到声明）/ `OrphanActor`（声明没有行指向）/ `ActorCannotClose`（`writes ∩ 展开后的 until∪unless = ∅`）/ `ActorModeTooNarrow`（`decl.tier > actor.mode`，反射档拿不到模型句柄）四格；`fact_names_of` 把视图源折成它 `reads` 的事实名再求交，否则 `execute-turn`（`until = turn_closed` 是视图）假红。④ **动作面**：`turn_exec` 长出 `TurnStep`（方案的 `ActorCtx`：本轮草稿 + 七个端口）/ `StepFuture`（**一条**寿命 + `Send`——`dyn Future + 'a + 'b` 非法，rustc E0226，这是 §2.3 草图 `'static` 换不来的那条）/ `Action` / `run_execute`；`Executor` enum + `for_by` 名字 match + `accepts` 谓词整块删除，派发点只剩 `(next.action)(&mut step).await`。⑤ **行表**：11 行显式 `by: None`（`authorize-tool` / `policy.classify` / `policy.report` / `policy.compress` / `policy.recall` / `policy.reflex` / `dispatch-turn` / `dispatch-model` / `open-tools` / `dispatch-tools` / `lease`，各带一行「未接线」理由），`execute-turn` 用 `Some(EXECUTE)`；`ACTORS: &[ActorEntry]` 是 `decl` 与 `run` 的同批绑定，`actor_decls()` 是它的 `decl` 投影。⑥ **取值不变性核对**：`owes()` 重定义为纯 `open` 后，12 个生产点问（`chat_loop.rs:463,589,124`、`chat_loop/inputs.rs:163,327`、`port.rs:569`、`tool_dispatch.rs:288,424,490`、`turn_exec/assemble.rs:236`、`turn_runner/stream.rs:75`）问的行 `by` 都从 `Some` 翻来、`unless` 全空 ⇒ `blocked` 恒假 ⇒ 逐点同值；`await-*`（本来 `by = None`）无人点问，只有 `drive` 会跳过它们。⑦ **停机判据没搬家**：`count_model_step` 拆成 `record_model_step`（记，在执行体里）+ `over_step_limit`（判，在派发器、动作之前），ADR-118 决策 7 保持。⑧ **用例**：`ring.test.rs` 新增 4 条（`a_by_without_an_actor_is_a_load_time_gap` / `a_none_by_needs_no_actor` / `an_actor_that_writes_nothing_on_the_exit_is_a_gap` / `a_row_slower_than_its_actor_mode_is_a_gap`）、删 `accept_skips_rows_without_an_executor`；`loop_rows.test.rs` 新增 2 条（`next_hands_back_the_action_itself` 用 `std::ptr::fn_addr_eq` 钉住派发器拿到的就是 `run_execute`、`the_row_table_and_the_binding_table_agree` 对真表双向对账），`prio_is_global_...` 改由 `wired()` 造「同时可还」的表（`accept` 没了之后真表只有 `execute-turn` 可选，裁决序得靠构造），`owes()` 本地辅助重定义为纯 `open` 并把「未接线行也算欠」写进注释。⑨ **熔断线口径**：非注释生产代码 `+259 / -99`（`loop_rows.rs` 82 / `turn_exec.rs` 71 / `decl/mod.rs` 47 / `ring.rs` 54 / `turn_runner.rs` 5），`turn_exec.rs` 那 140 行里 69 行是被删的旧形状——PL-016 §5 的 300 行按结构性改动计，本批在线内。⑩ **CLI 实测**（`--provider LMStudio`，本地 `lfm2.5-24b-a2b-apex`）：`verify-actor-1` 两幕（纯问答 + 工具轮）与 `verify-actor-2` 单幕工具轮，`exit=0`、`WARN/ERROR` 各 0；日志里 `[Loop] 派发 execute-turn（执行体 execute；债 turn=1）` 出现 1（actor-1 第二幕 3 次）/ 5（actor-2）次，工具段 `open-tools → authorize-tool → dispatch-tools` 三笔债在同一执行体内部依次可还并被 `vdfs_read` 真实执行；`node scripts/obligation-ring-accept.mjs --strict` 对两个会话 J1…J8 全绿（`verify-actor-2`：HTTP 4 = `sent(model)` 3 + 未入账判决 1，豁免条目即 T-247）。中途一次 J8 红已归因**不是代码**：把跨两幕的累积 WAL 对上单幕日志，`HTTP 5 / sent 3 / 判决 2` 合起来即平——实测脚本的 `--cli-log` 必须与 `--session` 同覆盖域。⑪ **文档**：新增 [ADR-119](../010-adr/119.md)（声明面/动作面分家的裁决与被否决方案）、PL-016 §2.3 的 `ActorDecl` 草图按它改写、`T-233` 处理过程里旧 `accepts` 谓词的失效指认按 D-005 历史行口径补上退役归属、`run` 的值返回账本连同「`Err` ⇒ 引擎写 `failed`」与判据 14 夹具登记为 `T-249`。⑫ **门禁**：`node scripts/gate.mjs` 129/130（唯一 ⊘ = release 二进制构建戳新鲜、无需重建），含 e2e 78/78；`cargo test -p symbio --lib` 2051 绿（棘轮自动上调 2046→2051）；`clippy --all-targets -D warnings` 绿；`registry-audit` / `doc-link-audit`（D-004 命中 0）/ `doc-symbol-audit` / `port-surface-audit` PTS-001…004 / `core-shape-audit` CS-001…002（traits 15<16、methods 55<56 的**低于基线**提示是本批之前就有的余量，未顺手放宽）。'
---

## 内容

父任务：[T-236-义务环符合性收口批](236-义务环符合性收口批.md)。

2026-10-22 对账：`ActorDecl` / `ActorError` 全仓不存在，执行体是裸函数 +
名字串匹配（`turn_exec.rs:892-910` 仅 `"execute"` 一档）；执行体返回
`Result<TurnOutcome>` 且**自己 append 事实**，与 §2.3「执行体交回若干事实、
引擎追加」的契约相反。§3.2 表明的三样静态检查（mode 闸门 / reads 存在 /
writes 封闭）因此全部失效。本任务是 [T-242-装载检查余四道](242-装载检查余四道.md) 与
[T-243-Builder六口与按会话seal](243-Builder六口与按会话seal.md) 的前件（检查 ⑤⑥ 需要 `ActorDecl.writes`）。

## 处理过程

- 2026-10-22 登记（PL-016 三面对账：方案 → 代码 → CLI 实证，见 T-236 内容节）。
- 2026-10-23 按 [ADR-119](../010-adr/119.md) 收窄本任务边界并拆出一件：§2.3 草图里
  `run` 的 `'static` 盒子与 §2.6 契约 1 的 `&mut Session` 在类型上互斥
  （`dyn Future` 只允许一条显式寿命上界，rustc E0226；core 里也没有 `Fact` / `ActorCtx`
  可拼那个签名）。裁决落法：**声明面进机制层，动作面留部署层**，两侧由
  `UnmannedRow` / `OrphanActor` 双向对账绑住。`run` 的值返回
  （`Result<Vec<Fact>, ActorError>`）连同「`Err` ⇒ 引擎写 `failed`」与判据 14 的空账本
  夹具，一起归 [T-249-执行体值返回账本](249-执行体值返回账本.md)——混进本任务会顶穿
  PL-016 §5 的 300 行熔断线。
