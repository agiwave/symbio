---
kind: task
id: T-321-B3-装配清单与身份由-manifest-生成
title: Phase B3：插件身份、路由与装配清单由 manifest 生成
status: doing
created: 2026-10-26
track: impl
prio: next
deps: []
parent: T-316-内核边界与扩展局部性收口
where: symbio/src/plugins/ · symbio/src/assembly.rs · scripts/
exit: '新插件 = 插件目录 + `PLUGIN.yml` + 声明；`PLUGIN_ID_*` / `ROUTE_*` / `ASSEMBLY_*` 由生成器产出（手写文件消失或手改 = 0）；加插件时 `symbio_core` diff 为空。'
---

## 内容

当前 `symbio/src/assembly.rs` 手写 21 个 `PLUGIN_ID_*` 与两条清单。本任务把它们移到
每个插件目录的 `PLUGIN.yml`，由脚本生成装配方常量；容器聚合，core 不枚举插件。

1. 定义 `PLUGIN.yml` 的最小 schema：id、meta、routes、装配位（是否默认挂载 / 是否可停用）。
2. 生成器产出装配方文件；生成文件带「不要手改」头与 `--check` 模式。
3. 删除手写字面量；加一个插件只改目录与其 `PLUGIN.yml`。
4. 反向用例：从 `PLUGIN.yml` 删掉一个插件，生成器红；装配树按新清单构造。

## 处理过程

- 2026-10-26：立任务。D-174 已把身份移出 core，本任务把它从手写常量变成声明生成。
- 2026-10-26：**B3a（身份 + 装配清单）已落**——21 个插件目录各建 `PLUGIN.yml`
  （`plugin_id` / `plugin_comment` / `plugin_assembly` / `plugin_order` / 可选
  `plugin_undisablable`），`scripts/facts/assembly.mjs` 生成
  `symbio/src/assembly.generated.rs`；`assembly.rs` 改成一个 `include!` 加原有模块文档，
  三组手写常量消失。facts 链路自动接入门禁（`gen-facts` 目录即接线），
  `facts/assembly.test.mjs` 覆盖缺声明 / id 不一致 / 顺序重复 / 手改生成物四个方向；
  `current.mjs`、`agent-ownership-audit`、`protocol-mirror-audit`、`plugin-entry-audit`
  的装配方路径与夹具同步到生成物。`node scripts/gate.mjs` 150/150。
- 2026-10-26：**B3b（`ROUTE_*`）待做**——8 条路由常量仍手写在
  `symbio_core/plugin/route.rs`，且各带调用方 / 语义注释；生成前先决定注释的 owner
  （路由地址归插件声明，语义说明归插件文档），避免把注释复制成第二份事实。
