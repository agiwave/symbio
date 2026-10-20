---
kind: task
id: T-201-VDFS单源与物理层收紧批
title: VDFS 单源与物理层收紧批——死字段删除、发现链路单接、默认收紧
status: done
created: 2026-10-20
track: plan
prio: next
deps: []
parent: T-183-质量与机制清偿总批
where: symbio/src/plugins/vdfs/ · symbio/src/plugins/session/plugin/vdfs_provider.rs · symbio/src/plugins/composite/vdfs.rs · docs/006-ref/001-CONFIGURATION.md
evidence: '四条落地：① `VdfsRequest::List` 删去 `limit` / `before`（参数袋保留），前端 `vdfs.ts` 同步；② 发现链路选单接——`Composite::traverse` 从 `Plugin::get_vfs_provider` 派生挂载行，13 个插件的自注册删除，新增 `mount_rows_are_derived_from_providers`，事实表挂载清单改由同一判据提取；③ 物理层选审计次选——`ToolVdfs` 的写 / 编辑 / 删除 / 建目录对工作目录外的绝对地址记 `plugin_warn!`（`audit_outside_workdir`），新增 `audit_flags_only_absolute_changes_outside_workdir`，不翻 `workspace_only` 缺省（配置面无白名单入口，ADR-101 同一取舍）；④ `etag` 选删字段——core 类型、响应字面量、tauri schema / service 与文档描述全部移除。验证：cargo test -p symbio --lib plugins::vdfs = 63 passed；全量门禁 124/124；e2e 78/78。'
exit: '有界列表的窗口字段（`limit` / `before`）从请求枚举删除，参数袋保留，有界列表收敛为单通道；发现链路二选一落地——容器从子插件派生挂载行把双接变单接，或加审计断言「凡能交出资源提供者的插件必有挂载行」，不得留「双接静默」形态；`workspace_only` 缺省改为开启（白名单根可配）或至少给 LLM 链路的写 / 删操作加审计日志，现有合法调用点先梳理；`etag` 二选一（物理层用 mtime + size 实现最小乐观并发并在冲突时报冲突错，或从内容与写响应里删字段）；全量门禁与 e2e 全绿'
---

## 内容

父设计：03 报告建议 1 / 2 / 3 / 4（该报告给 VDFS 的评级是 **A**，这四条都是「A 里剩下的债」）

**为什么 A 级机制还要动**：VDFS 是全仓质量最高的机制（单方法契约 + 编译期穷尽 + 单一 owner），
它的债都是**小而具体**的，但每一条都是「同一个事实两个 owner」的雏形：

| 项 | 现状 | 后果 |
|---|---|---|
| 有界列表窗口参数 | `List` 的 `limit` / `before` 在转发层就被丢掉 | 一个正在形成的双 owner 漂移点（字段在，没人读） |
| 发现链路 | 容器侧与插件侧**手工双接**（挂载行与资源提供者各注册一次） | 漏一处 = 静默不可达，无人会红 |
| 物理层权限 | `workspace_only` 缺省关闭 | 「全盘可读」是默认值，LLM 误写工作区外文件无迹可查 |
| `etag` | 类型里有，没人实现也没人消费 | 纸面能力（「没人调用的能力比没有更糟」） |

**四条的共同判据**：**同一个事实只能有一个 owner**。窗口参数的事实是「谁在限制」，
发现链路的事实是「谁能提供资源」，权限的事实是「能碰哪里」，`etag` 的事实是「并发怎么判」——
每一条今天都有两个答案或零个答案。

**两处「首选 / 次选」的取舍写在这里**（原报告给了两案，本批不预先裁决）：

- **发现链路**：首选「容器从子插件派生挂载行」（把双接变单接，结构上不可能漏），
  次选「加审计断言」（成本更低，但漏的时候仍是运行时才发现）。首选动装配时序，次选只加一个脚本。
- **物理层权限**：首选「缺省收紧 + 白名单可配」，次选「至少给写 / 删加审计日志」。
  前者是 fail-closed 方向（与本仓治理面一致），但要先梳理现有合法调用点
  （工作区外的读，例如临时目录，需要走配置）。

**不新增机制**：四条都是删字段 / 收紧默认 / 加断言，机制键 0、trait 0。

## 处理过程

- 2026-10-20 立批。来源：03 报告建议 1 / 2 / 3 / 4。
- 2026-10-20 原报告的第 5 条（复杂 provider 路由骨架）**不做**——只有 1 个实例时抽象是过早的，
  理由记在转化台账 §7-3。
- 2026-10-20 顺序建议：先 1（删死字段，纯减法，零风险），再 4（`etag` 二选一），
  再 2（发现链路，动装配时序），最后 3（权限收紧，要梳理调用点）——风险从低到高。

## 结果

四条全部落地，均为**删字段 / 单接 / 加审计**，机制键与 trait 数 0 变化：

| 项 | 落地 | 证据 |
|---|---|---|
| 有界列表窗口参数 | `VdfsRequest::List` 删去 `limit` / `before`，参数袋保留 | `symbio_core/vdfs/request.rs`；前端 `vdfs.ts` 同步；窗口字段无读取方 |
| 发现链路 | 选**首选**：`Composite::traverse` 从子插件 `Plugin::get_vfs_provider` 派生挂载行，13 个插件的自注册全部删除 | `composite/composite.rs`；`mount_rows_are_derived_from_providers`；事实表挂载清单改由同一判据提取 |
| 物理层权限 | 选**次选**：LLM 链路的写 / 编辑 / 删除 / 建目录加工作区外审计（`plugin_warn!`，记但不拦） | `vdfs/provider.rs` 的 `audit_outside_workdir`；`audit_flags_only_absolute_changes_outside_workdir` |
| `etag` | 选**删字段**：类型、响应字面量、前端 schema / service、文档描述全部移除 | `VdfsContent` / `VdfsWriteResponse`；`tauri/src/schemas/vdfs.ts` |

**权限项为何不翻缺省**：`workspace_only=true` 会改变地址规则里已承诺的绝对路径语义，
而当前配置面没有「白名单根」的入口；在拿到真实使用分布之前收紧，等于替用户做决定
（与 ADR-101 对本地策略的取舍一致）。因此本批按 exit 的次选执行：**变更动作可追溯**，
读数不进审计（审计要回答「改了什么」，不是全量访问日志）。缺省收紧留待有配置入口与
使用分布之后单独裁决。

**审计口径**：仅绝对地址可能落在工作目录之外（相对地址恒在工作目录内，`..` 在分流阶段
即被拒），因此审计判定只看绝对地址的前缀，不重复物理层的路径守卫。
