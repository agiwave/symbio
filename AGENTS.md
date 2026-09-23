# 工作区指令（本仓库特有）

## 结构事实：先查再断

[`docs/CURRENT.md`](docs/CURRENT.md) 由 `scripts/gen-current-facts.mjs` 从代码提取，
每次重生成，不会漂移。断言下列内容前读对应小节，不要凭记忆复述：

- 插件 / 挂载点 / 路由 / 配置文件 → §1
- LLM 可见工具名 → §2 ｜ 前端路由与 VDFS 操作 → §3
- 存储层（仅 `vdfs_service` 三实现，无多后端选型）→ §4
- 代码规模、Tauri 命令面、CLI 选项与命令 → §5

改完代码重跑生成脚本（CI 用 `--check` 防漂移）。「为什么这样设计」看
`docs/DECISIONS.md`，不要塞进 CURRENT.md。

## 压缩快照不是事实来源

快照只承载**只有本对话知道的东西**：偏好、已做的决策与被否决的方案及理由、死路、
工具怪癖。其中的**仓库结构一律视为过期**——它以权威口吻出现，最容易误导下一轮。
需要结构事实就回到 CURRENT.md 或直接读代码。

## 文档：一个事实只有一个 owner

改代码时**不要**顺手改一堆文档——先确认那条事实属于谁（职责边界表见
[`docs/README.md`](docs/README.md#文档职责边界一个事实只有一个-owner)）：

- 结构事实 → `CURRENT.md`（自动生成，不手改）
- 「为什么这样设计」→ `DECISIONS.md`（只增 ADR，不在别处复述）
- 路由 / 错误码 / 配置 → 各自的 `ROUTES.md` / `ERROR_CODES.md` / `CONFIGURATION.md`
- 模块机制 → 该模块 `README.md`
- **现行文档不写变更史**（「曾经…已改为…」）——那是 `git log` 与 ADR 的事

复述即重复：同一事实写两处，改一次要动两处，且必然漂移。见
[CONTRIBUTING.md §4](CONTRIBUTING.md) 的注释边界。
