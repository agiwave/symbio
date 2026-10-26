---
kind: task
id: T-323-C-实现搬迁与-crate-边界
title: Phase C：实现搬迁、actor 策略迁移与 crate 边界
status: doing
created: 2026-10-26
track: impl
prio: now
deps: []
where: symbio/src/symbio_core/ · symbio/src/plugins/ · Cargo.toml
exit: '可选批次：按 owner 搬投影实现与共享类型、把 schedule / skill / conation / commitment / recall 策略迁回 owner、`fire_hook` 与 `ROUTE_*` 一起搬出 core、拆出 `symbio_kernel`；C-002 的文本审计被 cargo 依赖方向取代。'
---

## 内容

只有 A/B 稳定后才做，不是前置条件：

- C0：**已判定无可搬条目**（D-187）：`node scripts/core-export-audit.mjs --verbose`
  的 `[L1]` 段是**候选报告**——13 条逐条核过，全部命中配对契约 / 单一统计口径或机制
  状态 / 跨 crate API / 闭集词汇·签名·安全面之一，搬走要先把私有面提成根出口或切出
  第二份口径，与「纯机械」的前提相反。判据面同批修正：`#[cfg(test)] mod <stem>;`
  声明的整文件测试模块算测试（`EventStore` 因此不在清单里）。
- C0.5：`fire_hook` 与 `ROUTE_*` 常量一起搬出 core（D-184）——`fire_hook` 是
  session / loop 共用的机制助手，而 `ROUTE_HOOK_FIRE` 是它唯一的 core 依赖；两者
  分开搬会得到「core 依赖 host」或「core 内藏字面量」两者之一，都是漂移；
- C1：actor 策略迁移；跨域的 control / preemption 单独论证；
- C2：拆 `symbio_kernel`，插件直接依赖 kernel；根账本消失或只剩 prelude。**抽取单位是依赖
  闭包，不是 `core-kernel-files:` 文件集**（D-188）：
  - C2a：新建 `symbio_kernel` crate，装已闭合子集 `event/` · `view/`（`Budget` / `View`）·
    `store/` · `projection/`（只 `Projection`）· `decl/` · `adapters::LatencyTier`（先拆独立文件）；
    core 依赖 kernel，`kernel.rs` 从 kernel 转出这批符号，对外路径不变。
  - C2b：五个 trait 及其签名类型搬入 kernel；`ActorSpec` 的授权读法改 core 侧自由函数
    （`authz` / `governance` 是部署表，不进内核）；插件改 `use symbio_kernel::…`。
  - 判据：C-001 / C-002 在 kernel 内由 cargo 方向取代；CS-001 / CS-004 根目录改
    `symbio_kernel/src`；CS-003 量两个 crate。
  - 每步 `cargo build -p symbio` → `node scripts/gate.mjs` → 提交。

## 处理过程

- 2026-10-26：立任务，置 later。D-183 明确「先模块后 crate」。
- 2026-10-26：接收 B3 的剩余路由常量（D-184）：原计划在 B3 生成 `ROUTE_*`，但
  `ROUTE_HOOK_FIRE` 的唯一 core 消费方是 `fire_hook`，现在搬会形成 core → host
  的反向依赖。改为随 C0.5 一起搬。
- 2026-10-26：接收 B4 的两条 exit（B 阶段结构上不可达，D-186 / T-316 的收口记录）：
  「`core/mod.rs` 手改 = 0」归 C2（随 crate 边界消失），「删除只为可达而存的根导出」归 C0
  （C-002 下根出口是唯一通道，删一条 = 搬实现）。本任务因此与 A/B 父任务解耦，独立成立。
- 2026-10-26：C0 收口为 **0 条可搬**（D-187）。`[L1]` 的 13 条逐条核过：配对契约
  （`capability_resolve`）、单一统计口径 / 机制状态（`fallback_rate` / `slo_report` /
  `check_all` / `creator_ids` / `logger_init`）、跨 crate API（两个 `logger_*`）、
  闭集词汇·签名·安全面（两个 embedding 项、`EVENT_BUS_KIND_SYSTEM`、
  `text_truncate_bytes`、`branch_hide_unselected`）。同批修正消费方口径：
  `#[cfg(test)] mod <stem>;` 声明的整文件测试模块不算生产消费方（`EventStore` 退出清单）。
- 2026-10-26：C2 方向裁决（D-188）。逐文件核过 `core-kernel-files:` 的 10 个文件，
  确认**不是依赖闭合集**：闭包带进 `plugin` / `exec` / `chat` / `vdfs` / `adapters::LatencyTier` /
  `view` / `authz` / `governance` / `present::ReadingSection`。按文件集硬抽会循环依赖或把
  部署表塞进内核。改为 C2a（结构层，已闭合子集）+ C2b（契约层，trait 与签名类型）两步；
  trait 泛型化方案因 36 个文件持有 `Arc<dyn …>`、会牺牲对象安全与 F1 而被否。
- 2026-10-26：**C2a 已落**。新建 `symbio_kernel` crate（workspace 成员 + `symbio` 依赖），装
  `event`（含 `grid`）· `view`（只 `Budget` / `View`）· `store` · `projection::Projection` ·
  `decl`（含 `ring`）· `LatencyTier`；`symbio_core` 以同名私有模块 `pub use symbio_kernel::…`
  转出，既有路径 `crate::symbio_core::<域>::…` 与根账本不变。跨 crate 等价性把 `decl` 的
  `pub(crate)` 放宽为 `pub`（`Seq::new` 同理）。判据面：CS-001 / CS-004 合并读
  `symbio_core/kernel.rs` 与 `symbio_kernel/src/lib.rs` 两处文件集标记，CS-003 扫两个 crate；
  C-003 的 core 内消费方统计把 kernel 也算作 core 自己的实现；其余审计根目录同步
  （doc-count / doc-symbol / port-surface / panic / test-layout / facts）。门禁 152/152 通过。
- 2026-10-26：**C0.5 已落**。`plugin/route.rs` → `symbio/src/routes.rs`（`pub mod`，cli 跨
  crate 取 `ROUTE_EVENT_BUS_SUBSCRIBE`）、`plugin/hook.rs` → `symbio/src/hooks.rs`（私有）；
  `fire_hook` 的三个测试随迁 `symbio/src/hooks.test.rs`。`symbio_core` 根出口与 `plugin`
  域不再转出 `ROUTE_*` / `HookEvent` / `HookOutput` / `fire_hook`，插件改
  `crate::routes::…` / `crate::hooks::…`。判据面：C-004 非内核账本 279 → 270（9 条随域
  迁出）；E-012 / route-facts / protocol-mirror / routes.gen.ts 的常量源改宿主文件；
  E-013 的 loop 闭族补 `routes` / `hooks` 两个**宿主机制**根（不是兄弟插件实现，D-184）。
