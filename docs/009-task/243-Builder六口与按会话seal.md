---
kind: task
id: T-243-Builder六口与按会话seal
title: Builder 六口与按会话 seal——行/端口/执行体经注册口进表，LazyLock 单例退役
status: done
created: 2026-10-22
track: plan
parent: T-236-义务环符合性收口批
prio: next
deps: [T-241-ActorDecl执行体注册表, T-242-装载检查余四道]
where: symbio/src/symbio_core/decl/mod.rs · symbio/src/symbio_core/decl/ring.rs · symbio/src/plugins/loop/loop_rows.rs · docs/004-plan/verify/seal_consumes.rs · docs/010-adr/120.md
exit: 'PL-016 §3.2 六个注册口齐：`fact / view / actor / port / row / seal`（现为 3/6）；`loop_rows.rs` 的 static 行切片与端口切片改经 `row` / `port` 入口装载；「运行期不能注册是类型不是纪律」保持——`Session` 只持 `Arc<Frozen>`；进程级 `LazyLock` 单例改为按会话构造注入（或明示其等价性并给出类型依据）；判据 2「表里零代码」复查通过；全量门禁与 e2e 绿，单次提交 ≤ 300 行。'
evidence: '[ADR-120](../010-adr/120.md)（等价性）+ 单批提交，基线 HEAD `453277e2`（T-242 收口）。① **六口齐**：`Builder { facts, views, actors, ports, rows }`——五个 `&mut self` 口（`fact / view / actor / port / row`）+ `seal(self)`，方法面由 `the_six_registration_ports_are_the_whole_surface` 按**函数指针类型**逐口绑定，其中 `fn(Builder) -> Result<Arc<Frozen>, Vec<Gap>>` 这一格钉住「`seal` 按值消耗」——签名改成 `&mut self` 当场编译不过。② **行与端口改从注册口进表**：`loop_rows::load(rows)` = `Builder::builtin()` + `actor_decls()` + `PORTS` + 行切片 → `seal()`；`assemble()` 只做「装载 + §3.4 第 4 条报告」。原旁路 `check_rows` / `check_ports` 作为调用方入口**删除**（`CheckError`、`builtin_with` 一并删），检查跟着表走、`seal` 是装载的唯一出口。③ **一套 `Gap` 不是两套**：声明侧九格（重名 / 撞名 / 视图引用未声明事实或字段 / 执行体声明引擎独占格 / 端口重名）并入 `ring::Gap`，`ring.rs` 的「## 声明侧的格子（同一个类型，不分两套）」写明分界；§3.4 指认表第 3、5 两行改指「`seal()` 的声明侧 + `check_rows` / `check_ports`」，任何缺口都在同一个 `Vec<Gap>` 里、`Err` 先于任何表产出。④ **口径 2 的两面证据**：`docs/004-plan/verify/seal_consumes.rs`——正常档 9 条断言（口面签名 / 好表冻出 / 缺口先于表且一次报全 / 三场会话共享下 `Arc::get_mut` 恒 `None` / 侧表进不了 `Frozen`），`should_not_compile` 档三条**各自编译失败**且错误码逐条对上：(a) seal 后再注册 ⇒ E0382 use-after-move、(b) 经会话表调注册方法 ⇒ E0599 无此方法、(c) 独占取到 `&mut Frozen` 后直写表字段 ⇒ E0616 字段私有。刻意留的边界也写进源码：绕开 `Builder` 的进程级侧表**编译得过**，一级强制止于此，那条账归二级强制（port-surface 审计 + 判据 2）。棘轮随之上调：验证程序 16→17、反例档 3→4、断言 247→256。⑤ **等价性挂在 ADR 上而不是复述在注释里**：[ADR-120](../010-adr/120.md) 给出三条类型依据（输入全 `&''static` 程序文本 / `seal` 消耗自身 / `Frozen` 无 `&mut` 无内部可变性），并把**作废条件**写死（一旦出现按会话不同的表即作废、改按会话注入）；`loop_rows.rs` 的小节只指过去，不复述论证。⑥ **夹具改造的代价如实记下**：装载只有一个出口 ⇒ `ring.test.rs` 走 `declare / home / freeze / seal_gaps / decl_world / seal_protocol`、`mod.test.rs` 走 `cover / core_table / gaps_of`，11 处 `builtin_with` 换成 `core_table()`（16 条 builtin 事实逐条补归宿行）。两处用例被迫补脚手架：`eval_respects_declared_cost` 要 `cover(EVENT_SENT)`、`an_actor_cannot_claim_an_engine_cell` 要给引擎格补 cover 行——因为 `seal` 现在也跑第 7 条，裸事实必须有名归宿。方向是判据变严不是变松：`a_port_without_a_failure_exit_is_a_gap` 现在成对报 `OrphanFact` + `PortWithoutAnswerExit`，`the_request_cell_belongs_to_the_decider` 的断言是 `vec![]`（要求一份真表式完整声明），三处精确名单（`DependencyCycle` / `ActorWritesEngineCell` / 上述成对缺口）仍逐格 `vec![...]` 钉死，没有为迁就夹具放宽。`seal_finds_every_kind_of_gap` 改名自 `check_rows_…`，且因为 `declare` 不带归宿行，`Gap::OrphanFact` 仍会显形。⑦ **判据 2「表里零代码」复查通过**：`ROWS` / `PORTS` 是数据切片，读它们的只有 `load` 里那两个循环，没有按名字分支造行的代码。⑧ **不违规**：求解器字节未动（调用方改传 `frozen.rows()`）⇒ PTS-003 指纹仍是 `sha256:b4aa7dddba3e6958…`（与 T-241/T-242 同一枚基线），PTS-001…004 绿；PTS-002（`ring.rs` 里不许出现端口字面量）由 `FactDecl.port` 的字段名读出口维持；CS-001 三格 `traits 15/16 · methods 55/56 · defaults 18/18` 未变宽（`Builder` 不是 trait，无需 allow 行与变宽 ADR）。⑨ **熔断线**：非注释生产代码 core `+114 / −70`、`loop_rows.rs` `+34 / −22`，单批合计约 170 行，在 PL-016 §5 的 300 行线内；测试夹具、验证程序与判据脚本按 §5 属语义产出物，不计入。⑩ **CLI 实测两幕**（会话 `verify-seal-1`，本地 `lfm2.5-24b-a2b-apex`）：两幕各 `exit=0`、WARN/ERROR 各 0，共 58 格 / 10 组端口往返；开幕 info「`[Loop] 相交行对 8 组`」与 `the_overlap_report_on_the_production_table` 钉的名单逐字一致；两幕派发序列齐（`execute-turn` → `dispatch-model` → `open-tools` → `authorize-tool` → `dispatch-tools`），经新装载路径跑通即「六口 + seal 没误伤真表」。⑪ **复验判红的归因与订正**：`obligation-ring-accept.mjs --strict` 初判两项失效（J7 说豁免条目在本实测里没有观测、J8 报 `HTTP 7 ≠ sent(model) 5 + 豁免 0`）。逐格核对后确认**不是本次改动引起的行为差**，而是判据脚本的观测口径写错：`classify` 两幕各打了一次模型（日志两行 `[Classify] 分类使用模型` + 两次 `tools=0` 的 HTTP），而判决不可解析 ⇒ 落 `Escalate{unclassified}` ⇒ `turn.stage{decide, source: "fallback"}`——`source_of` 记的是判决**来源**不是「有没有打过模型」，这次调用在事实层零痕迹。脚本已订正：该路径的观测改读 CLI 日志（`MODEL_PATHS.logMarker`，日志切不出会话归属 ⇒ J7 日志口径与 J8 一并整轮判），`--strict` 下不给 `--cli-log` 判红而不是「未判 = 过」。复验结果 `HTTP 7 = sent(model) 5 + 豁免未入账 2`，`--strict` 全项通过（exit=0）。这条发现连同其对 T-247 三选一裁决的含义（入账动作必须发生在调用那一步本身，事后补记来源问不出「这次会不会打模型」）已带实测编号补记进 [T-247-插件内部模型往返进环](247-插件内部模型往返进环.md) 处理过程，本批不动契约。⑫ **门禁**：`node scripts/gate.mjs` 通过 131/132（唯一 ⊘ = release 二进制按源码指纹无需重建，非红项；同轮新增 `verify: seal_consumes` 与其反例两格均绿），含 e2e 78/78；`cargo test -p symbio --lib` 2066 绿（棘轮 2065→2066）；`clippy --all-targets -D warnings` 绿；`cargo fmt` 已应用。'
---

## 内容

父任务：[T-236-义务环符合性收口批](236-义务环符合性收口批.md)。

Builder 现缺 `actor / port / row` 三口，执行体与行散落在插件 static 数据里靠
`LazyLock` 汇聚——声明层（五张注册表 + seal 七检）与数据面（行表）中间隔着一层
未成文的胶水。补齐后 §3.3「三个时点」在代码上可指认。

## 验收口径

1. `Builder` 方法面 = 6（fact / view / actor / port / row / seal），有编译期或测试断言；`loop_rows` 的 static 切片不再是行的唯一进表通道（或明文论证等价性并挂 ADR）。
2. 「运行期不能注册是类型」：Session 构造后无可达可变注册口——`verify/` 下 `should_not_compile` 用例或等价编译断言。
3. CLI 复验两幕全过 + 门禁全绿；CS-001 / PTS-003 不违规（变宽需 allow 行 + ADR）。

## 处理过程

- 2026-10-22 登记（PL-016 三面对账：方案 → 代码 → CLI 实证，见 T-236 内容节）。
- 2026-10-23 收口：六口齐 + `seal` 成为装载唯一出口（声明侧与行侧共用一个 `Gap`），
  行/端口/执行体经注册口进表；`LazyLock` 单例按口径 1 的括号分支走「明示等价性 + 挂
  [ADR-120](../010-adr/120.md)」，类型面证据落 `verify/seal_consumes.rs`（三条 `should_not_compile`）。
  CLI 两幕复验中判出的 J7/J8 红归因为**判据脚本的观测口径**（判决落 `fallback` 时
  事实层看不见那次调用），订正后 `--strict` 全项通过；该缺口连同裁决含义移交 [T-247-插件内部模型往返进环](247-插件内部模型往返进环.md)。
  全量门禁 131/132（唯一 ⊘ 为非红项）、e2e 78/78、`cargo test --lib` 2066 绿。
