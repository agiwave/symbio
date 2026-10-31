---
kind: task
id: T-007-loop存量能力迁移
title: S6：loop 存量能力逐批迁出
status: doing
track: arch
prio: next
created: 2026-10-29
parent: T-001-能力全面拆分
deps: []
where: `docs/003-design/026-能力拆分与影响类.md` §2.3, `crates/symbio-plugin-loop/src/loop_rows.rs`, `crates/symbio-plugin-loop/src/turn_exec.rs`, `crates/symbio-plugin-loop/src/chat_loop`
exit: `crates/symbio-plugin-loop` 里按能力名存在的行、执行体、常量与策略清零——只剩环、写口、全局守门、过程体、会话基底五类；每一批都是「插件落好 → 绑定切换 + 旧实现删除」同一提交，端到端行为逐字不变；迁移提交不含机制增量（`EX-007`：`core` / `kernel` diff = 0，`loop` 只含删除与绑定切换）；`SC-006` 的基线里不再有 `loop` 条目
---

# S6：loop 存量能力逐批迁出

## 内容

前置的账（`DE-026` §2.3）与三样通用件（S3–S5）补齐后，`loop` 里的能力就能一段一段搬走。批次以 §2.3 的账为准
（名册在 loop 的 10 行：工作记忆、情景记忆、语义记忆、程序记忆、长期记忆检索、
副作用分级、规划与重规划、技能编译、反自动化回退、独立验证与返工）；另有三步
名册不单列（折在「多轮对话」里）但同样按名字存在于 `loop`，`SC-006` 的零工具名口径一并要求迁出：
语义压缩、判决 / 措辞、汇报产线。按「先无耦合、后带模型」排序，每批一个提交，
每批都带 T1 / T2 / T3 与一条反向用例（D-245）：

| 批 | 名册行 | 能力 | 今天在 loop 的样子 | 迁出后的形状 |
|---|---|---|---|---|
| L1 | 1 / 16 | 工作记忆 + 规划与重规划 | 轮首装配与计划寄存在本轮状态里 | 段走 I-2，跨步状态与计划走 I-1 / I-4 |
| L2 | 2 / 3 / 6 | 情景 / 语义 / 长期记忆检索 | #2 已落（L2a，D-263：编码 = 收束贡献，随 `symbio-plugin-memory` 的 `episodic` 子模块，D-267）；#3 / #6 待迁：`memory_events` 读侧 / `recall`（程序记忆 #4 已随 L3b 迁出） | 与 #2 同包（`symbio-plugin-memory`，D-267）；检索段走 I-2，折叠名字读 |
| L3a | 26 | 独立验证与返工 | `task_events` + 任务表写侧判断 + 收束时调度 + 就绪段 | `symbio-plugin-task`：写侧走 I-1（效果端口 + `before`），就绪段走 I-2（provider），收束时调度走收束贡献（D-250）；`rework_bounded` 不变量留核 |
| L3b | 24 / 25 / 4 | 技能编译 + 反自动化回退 + 程序记忆 | `SkillCompiler` / `SkillRouter`（原 `skill_events`） | 已落 `symbio-plugin-skill`（D-262）：`skill.recall` 行执行体（命中交 `RowStep::Answer`，摘行走 `skill.retreat` 名字读）+ 轮末收束贡献（编译）；两行同一实现、同包 |
| L4 | 8 | 副作用分级 | `breaker` | `before` 交拒绝（I-3），状态随包 |
| L5 | — | 语义压缩 | `COMPRESS` 执行体 + 轮首那一行 | 模型往返端口 + 通用省略事实（`Message` 摘要） |
| L6 | — | 判决 / 措辞与汇报 | `CLASSIFY` / `REPORT` 执行体 | 判定生产者走 I-1 / I-4；强制留引擎（D-240） |
| L7 | — | 续幕 / 装配 | `CONTINUE` / `SEAM` | 先判归属：只留「环的一步」；超出五类残留的搬走 |
| L8 | 11 | 多轮对话 | `chat_loop` 整轮循环 | 留环（D-222）；不迁，账里写明 |

每批的验收不是「代码搬了多少行」，而是**删掉了什么**：`loop_rows.rs` 里的那一条行 /
那一个 `ActorDecl` / 那一段按名字的分支不再存在，而端到端输出不变。

**迁移提交不含机制增量**（`EX-007` / D-247）。某一批若发现「还缺一条端口 / 一条分支才迁得动」，
说明前置设计不足：停下这一批，把补件单列为带 `### D-NNN` 的独立提交（通用、不含能力名），
再恢复迁移。不许在迁移提交里顺手在 `loop` 加行、加分支、加常量；也不许把能力实现搬进
`core` / `kernel` 或别的控制面模块来「满足」封闭集——那是反向拆分。

### 进度

| 批 | 状态 | 落点 |
|---|---|---|
| L2a 情景记忆编码 | 已落 | `crates/symbio-plugin-memory` 的 `episodic` 子模块（D-263 / D-267） |
| L3a 独立验证与返工 | 已落 | `crates/symbio-plugin-task` |
| L3b 技能编译 / 反自动化回退 / 程序记忆 | 已落 | `crates/symbio-plugin-skill`（D-262） |
| L2b 长期记忆检索 | 设计已定（D-264），未起 | `crates/symbio-plugin-memory` 的 `recall` 子模块 |
| L2c 语义记忆 | 设计已定（D-264），未起 | `crates/symbio-plugin-memory` 的 `semantic` 子模块 |
| L2b′ 巩固 | 设计已定（D-268），未起（排在 L2b 之后：复用包内检索折叠） | `crates/symbio-plugin-memory` 的 `consolidation` 子模块 |
| L1 / L4 / L5 / L6 / L7 | 未起（L4 还缺一条 `### D-NNN`：`breaker` 的判据要读授权矩阵与成本台账，两样都在 core，先裁「谁读、怎么读」再动手） | — |
| L8 多轮对话 | 不迁（D-222 留环） | — |

**L2b 起手前勘察出的六条约束**（省下一次重复勘察，都是这次实际踩到的；第 4 / 5 / 6 条
已由 `D-268` 裁决消失，原文与「为什么消失」都留着——那三条正是「前提错了会造出三个
假阻塞」的样本）：

1. **不新建 crate，落 `symbio-plugin-memory` 的子模块**（`D-267`）。记忆是一个所有者：
   文件形态（`AGENTS.md` / `WORKSPACE.md`）与事件形态（`memory.encoded`）是同一个
   所有者的三种载体，拆成四个插件之后没有任何一处说得出「记忆归谁」。`episodic`
   子模块已经并回去了，`recall` / `semantic` 照同一个形状落。
2. **扫描配额要在包内重算**。`Retrieval` 自带 `degraded`，意味着
   `RECALL_SCAN_QUOTA` 的截断语义（`Projection::apply` 的三行：配额取 `max(1)`、
   只截尾、`degraded = 窗口 < 全长`）必须由 `Retriever::retrieve` 自己实现一遍。
   省掉就是「空且不标降级」——DE-036 §6 验收 2 要消灭的那个形态。
3. **测试面比代码面大**。`memory_events.test.rs` 里有 15 条用例直接钉
   `recall_view` / `prompt_section` 的口径（新近序、查询语义维、跨会话只扫一层、
   降级措辞、同文去重、排除式遗忘）。它们随能力一起搬，不能删——那 15 条就是
   D-264 四条口径的验收，删掉等于把验收一起搬没了。
4. **~~`memory.live` 的读面缺两块事实形状~~ —— 已裁决（D-268），不再是补件**。
   原判断：`memory.consolidated` / `memory.forgotten` 没有 `FactDecl`，`ViewDecl.reads`
   只认已登记字段，所以 `memory.live` 声明不全依赖边。**前提已废**：活集折叠不作视图，
   是包内私有函数（`episodic::live_contents` 今天就是这么读的，收 `&[Event]`、直接读
   信封），`reads` 表根本不参与。两张形状仍要补，但补的理由换了——写方搬进包，
   形状随写方走，同一条迁移提交里补，不是前置补件。
5. **~~`memory.forgotten` 的排除判据读信封字段~~ —— 已裁决（D-268）**。`produced_by`
   在信封上、不在载荷里，`reads` 表声明不了——但那只是「折叠做成视图」的代价。
   折叠收的是 `&[Event]`，信封字段直接可达，这一格不存在了。
6. **~~巩固的失败方向要先裁~~ —— 已裁决（D-268）**。原问题：巩固改按名字读
   `memory.live` 之后，视图不可用时「跳过」还是「退回本地折叠」。**两者都不选**：
   巩固自己就在包里，输入是宿主交的全史切片，没有「视图不可用」这一支。

第 4 / 5 / 6 条同源，一次裁决全部消失（`D-268` 第 2 点）。L2b 因此回到「一次迁移」：
只剩上面第 1 条的落点约束、第 2 条的配额重算、第 3 条的测试面。巩固本身另起 L2b′，
**先一条补件**（门面加 `merge_with_fidelity` + 包里落 `consolidation` 子模块，不提交
`submit_consolidator!`），再一条迁移（注册 + 换绑 + 删旧实现）——`EX-007` 不许两类
混在一次提交里，而 `HS-003` 又不许门面那一条先于消费方落地。


**为什么不半途提交**：`D-245` 要求「插件落好 → 绑定切换 + 旧实现删除」同一提交，
`D-247` 禁止迁移提交夹带机制增量。两批各是一个原子批次，起手就必须走完；
把 crate 建好但 loop 未切绑定的中间态**不能进主干**（两份实现同时生效，`EX-003`
正是判它的反向用例）。

## 处理过程

1. 每批先在新 crate 落实现（未绑定，不生效），核对 `deactivate` 下系统完整运行。
2. 同一提交切换绑定并删除 `loop` 旧实现；不允许两份同时生效。
3. 跑该能力的端到端用例与 `EX-003` 反向用例（两份实现同时生效必须红）。
4. 如新增了工具名字面量，删掉后同批把 `SC-006` 基线对应条目删掉，确认红变绿不是靠改基线放宽。
5. 每批提交核对 `EX-007`：提交正文写一行 `Migration: <批次>`（判据按声明认迁移类），
   同一次提交里 `core` / `kernel` / `plugin-api` 非生成 `.rs` diff = 0，`loop` 的 diff
   只有删除与绑定切换；补件另起提交、挂 `Core-Change: D-NNN`。
6. 八批走完后核对 `loop_rows.rs` 只剩五类残留，再进 S7。

