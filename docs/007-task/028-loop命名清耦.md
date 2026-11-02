---
kind: task
id: T-028-loop命名清耦
title: loop A 类命名清耦：端口词走常量、行名单数化
status: doing
track: arch
prio: now
created: 2026-11-02
parent: T-001-能力全面拆分
deps: []
where: "crates/symbio-plugin-loop/src, crates/symbio-kernel/src/decl/ring.test.rs, crates/symbio-plugin-session/src/docs/102-core-loop.md"
exit: 生产代码里的端口名字面量（`"model"` / `"tool"`）与 `model-` / `tool-` id 前缀全部改走 `ports::MODEL` / `ports::TOOL` 常量（字符串值不变，e2e 逐字不变）；行名 `dispatch-tools`→`dispatch-tool`、`await-tools`→`await-tool`（open-tools / deny-tools 是批次语义，保留复数；行名在 e2e / PTS / verify 基线无读点）；`PORTS` 表与 `Lit::Str` 模式位点同批走常量；注释与模块文档同步；门禁全绿
---

# loop A 类命名清耦：端口词走常量、行名单数化

## 内容

[T-026] A 类清单的落地批：loop 层不区分 Model / Tool，端口词只有一份拼写
（`symbio_plugin_api::port::ports`）。只改命名与常量替换，不改任何行为——
常量替换的字符串值与原字面量逐字相同（`ports::MODEL` = `"model"`，
`ports::TOOL` = `"tool"`），行名改名已核无 e2e / PTS 读点。

## 处理过程

1. **id 前缀参数化**（`port.rs`）：`Round::id` 的 `model-{turn}-d{drive}-{step}` 与
   `control_decl.rs` 的 `model-{turn}-d{anchor}-s{step}`、`tool_dispatch.rs` 的
   `batch_call_id`（`tool-{turn}-d{drive}-{round}-…`）与 `latest_model_response`
   的 `model-{turn}-d{drive}-` 前缀，全部改由 `ports::MODEL` / `ports::TOOL` 拼出。
2. **配对走常量**（`control_decl.rs`）：11 处 `str_field(e, "port") == Some("model"/"tool")`
   与 `serde_json::json!` 里的 `"port": "model"` 改走常量（FactDecl 字段名列表里的
   `"model"` / `"tool"` 是**字段名**，不动）。
3. **`PORTS` 表与行模式**（`loop_rows.rs`）：`PortDecl.port` 两格与 24 处
   `("port", Lit::Str("model"/"tool"))` 模式位点改走常量。
4. **行名单数化**：`dispatch-tools`→`dispatch-tool`、`await-tools`→`await-tool`
   （loop_rows 行名 + tool_dispatch 的 `row:` 位点 + 全部测试断言与注释）；
   `port.test.rs` 的 `every_port_row_is_in_the_table` 去掉 `stem = "tools"` 特判，
   行名一律 `dispatch-{port}` / `await-{port}`。kernel `ring.test.rs` 的历史举例与
   session `docs/102-core-loop.md` 的行表同步。
5. **排除项复核**：`stage_events.rs` 的 `Self::Model => "model"`（阶段词汇）、
   `tool-resume-` 前缀（恢复幕词汇）、载荷字段名 `"tool"` 均非端口词，不动；
   测试夹具里构造载荷的 `"port": "model"` 字面量是模拟线上数据，不在清耦范围。
6. **验证**：`cargo test -p symbio-plugin-loop` 467 过（数不变）；全量门禁见提交。
