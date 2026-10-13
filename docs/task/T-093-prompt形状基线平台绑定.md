---
kind: task
id: T-093-prompt形状基线平台绑定
title: t46 形状基线把「平台命名的 shell 工具」（Windows `cmd` / Linux `sh`）钉进了判据——基线只能在捕获它的那个平台绿
status: done
created: 2026-10-13
track: e2e
deps: []
prio: now
where: docs/ref/PROMPT_SHAPE.md（基线本体）+ scripts/prompt-shape-audit.mjs（PS-001 工具面判据）+ e2e/cases/t46-prompt-shape-baseline.mjs
exit: t46 在 Windows 与 Linux 上**同时**可绿（基线不再把平台命名的工具当结构判据，或按平台分别持有基线）；若为此改了 ADR-057 的判据口径，变更写进该 ADR
evidence: 取候选 1（归一化平台变体）。`scripts/prompt-shape-audit.mjs`：新增 `PLATFORM_VARIANT_TOOL = '<shell>'` 与 `PLATFORM_VARIANT_TOOL_NAMES = ['cmd','sh']`，`extractShape` 对工具名逐位过 `normalizeToolName`（判据仍记**顺序、数量与其余名字**）；`SCHEMA_VERSION` 1→2，`diffShapes` 在 schema 不一致时**直接判漂移并提示重生成**（不再产出一串看不懂的字段差）。`scripts/prompt-shape-audit.test.mjs` 补 3 条用例（合计 23）：① 名单**逐字**钉 `shell.rs` 的 `get_os_info().tool_name`（漂移即红，不留第二份真相）② `extractShape` 把每个平台变体归一成记号、对真名叫 `shell` 的工具不动 ③ `diffShapes` schema 不一致只回一条含「重生成」的差。`docs/ref/PROMPT_SHAPE.md` 重生成：`schema: 1→2`、三处工具名 `cmd→<shell>`，其余逐字段不变（证明归一化后形状平台无关）。`docs/adr/ADR-057.md` 决策 2 的工具面条款与「后果」第 6 条记录本次补全。验证：`node --test scripts/prompt-shape-audit.test.mjs` 23/23 绿；`E2E_CASE_SELF=1 node e2e/cases/t46-prompt-shape-baseline.mjs` 绿（与重生成的基线逐字段一致）。
---

## 内容

CI 的 `e2e-check` 恒定红在 `t46-prompt-shape-baseline`（run 38372186256：48/50；run 38358236876：49/50），
而本机（Windows）跑整轮门禁是 50/50 全绿。红的形式是
`assertEq(a.status, 0)` ⇒ `prompt-shape-audit.mjs` 以 PS-001「形状与基线不一致」退出 1。

## 诊断（2026-10-13）

`docs/ref/PROMPT_SHAPE.md` 的判据 JSON 里，三个请求的工具面都是
`agent_run,cmd,content_search,todo_write,…`（工具数 21）——**其中 `cmd` 是 Windows 的 shell 工具名**。

`symbio/src/plugins/local/shell.rs:95-124` 的 `tool_name` 按 `target_os` 取值：
Windows = `cmd`，macOS = `sh`，Linux = `sh/bash`（取 `sh`）。于是 Linux 上
`diffShapes`（`scripts/prompt-shape-audit.mjs:214`）判「工具面：新增 [sh] 移除 [cmd]」⇒ PS-001 红。

旁证两条：① 基线自述「重生成于 2026-10-12」——在 Windows 上重生成；② 2026-10-08 的
CI run 37721749855 里 `e2e-check` 是**绿**的，与「那份基线此前是在 Linux 上捕获的」一致。

**结论**：t46 的基线是**平台绑定**的（它把「平台命名的工具」当成了结构判据），
因此「哪台机器最后重生成，CI 就在另一台机器上红」——这条守卫在 CI 上目前无法执行其职责。

## 处置方向（登记时待定，现已定：取候选 1）

三条候选，需择一并写明理由（**结论**：取候选 1，见文末「处理过程」末条）：

1. **归一化平台变体**：`extractShape` 把平台命名的 shell 工具映射到一个规范记号（如 `shell`），
   判据仍记「顺序 + 计数 + 其余名字」。改动最小，且**平台无关性由构造保证**（归一化后的形状
   在两平台上逐字段相同），本机即可验证。
   ⚠️ 障碍：归一化要有一份「哪些名字是 shell 工具」的**名单**，而这份名单会与
   `plugins/local/shell.rs` 的 `target_os` 取值链漂移——本仓对这类第二份名单有明确态度
   （见 `55-verify.mjs`「反例能力由源码自己声明，不维护第二份名单」）。要选这条，
   须先解决「名单从哪来」：要么由源码声明（如 shell 工具自带一个 `platform_variant` 标记），
   要么接受一份带守卫的名单。

2. **按平台分别持有基线**：基线文件以 `场景 + 平台` 为键存多份，各平台与自己的那份比。
   判据最强（每个平台的形状都被钉住），且**不需要任何名单**。
   ⚠️ 障碍：Linux 那份只能在 Linux 上捕获，本机（Windows）无法生成 ⇒ 须在 CI 上生成并提交。

3. **让 shell 工具名平台无关**（生产侧：统一暴露为一个名字，平台差异下沉到执行时）。
   这是唯一「一处修、两平台都不需要名单也不需要多份基线」的方案，且顺带消掉一个泄漏
   （模型不该从工具名看出宿主平台）。
   ⚠️ 障碍：它是**生产行为变更**（模型看到的工具名会变，`cmd` 相关引用需全仓对齐），
   属产品面的决定，不宜随本任务顺带做。

必要性论证（三条共有）：现状下 t46 在 CI 上**恒红且红得无信息**（报的是「形状漂移」，
真因是平台），这属于本仓反复记录的「守卫看起来在跑、其实判不了」形态；不修则
`e2e-check` 永远不可能绿。

口径归属：ADR-057 明写「判据口径（什么进基线、什么只作观测、为什么规模不进判据）」归它。
但三条候选里 **1 与 2 其实都符合 ADR-057 已有的原则**——它钉的是「**装配决定**的结构」，
而 shell 工具名是**平台决定**的（与「规模由用例文本决定」同族，故同样不该进判据）。
所以方向 1/2 更像对该原则的一次补全，而非推翻；落笔时按这个角度写。

## 不是本任务范围

- 不改 `shell.rs` 的工具名（那是**生产行为**：模型看到的工具名会变，且 e2e / 文档有引用）。
- 不追 `t36-autonomous-initiator` 的一次失败（run 38372186256，1m41s 超时形态）——它在更早的
  run 里是绿的，形态更像抖动；若再复现，另立任务。

## 处理过程

- 2026-10-13 登记：清偿 `v2` 分支 CI 全红时，逐 job 排查到 `e2e-check`；本地 e2e 50/50 全绿 ⇒
  定位为平台差异；读基线判据 JSON 见 `cmd`，对照 `shell.rs` 的 `target_os` 取值链得出上述结论。
- 2026-10-13 复核：本会话末推送后 CI run **38389418305** —— 9 个 job 里 **8 个绿**，
  **只剩 `e2e-check` 一项红**（49/50，失败用例恰为 `t46-prompt-shape-baseline`），
  即本任务是目前 `v2` 分支唯一未清的 CI 红；同 run 里 `t36-autonomous-initiator` **未复现**
  （印证它是抖动，不是稳定红）。
- 2026-10-13 补分析：三条候选各自的障碍（第二份名单 / 只能在 Linux 生成 / 属生产行为变更）
  已写进「处置方向」，并指出 1 与 2 与 ADR-057 既有原则同向——留给决策者择一，本会话不动口径。
- 2026-10-13 落定（候选 1）：**归一化平台变体**。择它的理由：① 它是唯一**本机即可验证**的方案
  （归一化后形状在两平台上逐字段相同，平台无关性**由构造保证**，无需在 Linux 上捕获第二份基线）；
  ② 它朝 ADR-057 决策 2 的既有原则**收敛**而非改写——判据本就只该记「装配决定的结构」，平台名不属此列；
  ③ 它的障碍（第二份名单）用**逐字钉源码**消掉：`prompt-shape-audit.test.mjs` 直接读
  `shell.rs` 的 `get_os_info().tool_name` 与 `PLATFORM_VARIANT_TOOL_NAMES` 对表，名单不是「又一份真相」，
  而是源码的一处投影，漂移即红。候选 2（多份基线）需在 CI 上生成并提交 Linux 那份，把「平台无关」
  变成「平台各管各的」，与判据口径的收敛方向相反；候选 3（改生产工具名）是产品决定，不在本任务范围。
  实现见 `evidence`。归一化记号取 `<shell>` 而非 `shell`：后者是一个**可能真实存在**的工具名，
  用它会把「真有工具叫 shell」与「平台变体被归一」混成一件事。
