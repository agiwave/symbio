# e2e 测试环境（CLI 模式）

以 `cli/`（进程内直连插件树）为被测前端，mock 与测试脚本全部为 Node `.mjs`，
Rust 侧只构建 CLI 二进制。mock 与被测系统之间走**真实边界**：
HTTP/SSE（LLM）、stdio/JSON-RPC（MCP）、磁盘（homedir）。

## 组件

| 文件 | 作用 |
|---|---|
| `mock-llm.mjs` | OpenAI Chat 兼容 Mock LLM：SSE 流式、场景编排（`match`/`afterTool`/`once`/`chunks`/故障注入）、请求记录 `GET /_requests` |
| `mock-mcp.mjs` | stdio JSON-RPC Mock MCP server：规范握手、可编排工具与错误/断连、调用记录 `--record` |
| `mock-actions.json` | mock-mcp 行为脚本示例（T6：`add` 返回 JSON-RPC 错误） |
| `helpers.mjs` | 夹具与断言库：临时 homedir、mock 进程编排、CLI 运行器、`defineCase`、共享不变量断言 |
| `cases/*.mjs` | **测试用例（每用例一文件）** |
| `cases/_selfrun.mjs` | 用例自执行引导（`_` 前缀 = 共享材料，不当作用例） |
| `run-tests.mjs` | runner：发现式加载 `cases/`，每用例独立子进程运行；开跑前确保被测二进制对应当前源码 |

## 运行

```bash
node e2e/run-tests.mjs                # 全量（被测二进制由机制保证最新，见下）
node e2e/run-tests.mjs t2             # 按名称过滤
node e2e/cases/t5-llm-http-error.mjs  # 单独跑一个用例（文件可直接执行）
E2E_DEBUG=1 node e2e/run-tests.mjs    # 失败时输出错误堆栈
```

**被测二进制不需要你记得重建**：`scripts/cli-binary.mjs` 是这条知识的唯一真相
（门禁与 e2e 共用）。判据是**内容指纹**——`cli/src` + `symbio/src` 的全部源码与
两个 crate 的清单算一个 sha256，构建成功后写成构建戳（`.symbio-cli.build-stamp`，
与二进制同目录）。使用前比对：戳一致就直接用；不一致就 `cargo build --release`
（cargo 自己判增量，源码没真变时 0.3 s 结束）再写戳。

为什么不能用「文件在不在」当判据：曾经就是这么写的，于是本机那份过期 exe 被一直
用下去，e2e 的失败形态与**眼前的源码直接矛盾**（源码里明明有的字段，运行时是
`undefined`），排查方向被引到源码上——而真因只是产物旧。二进制是构建产物的函数，
产物比输入旧就是不可信的。

构建必须在 `cli/` 下跑（仓库根没有 `Cargo.toml`；`cli/.cargo/config.toml` 会把
target-dir 指向 `../symbio/target`，二进制落在 `symbio/target/release/symbio-cli.exe`）。
机制按「两个候选取最新的那份」解析，所以两种布局都对。手工 `cargo build --release`
也可以，代价只是下次使用时会多跑一次 cargo（戳没更新）。

`E2E_CLI_EXE=<path>` 指向外部二进制会**绕过**新鲜度判定——外部产物无法用本仓源码
指纹衡量，这是知情选择。

诊断当前状态：`node scripts/cli-binary.mjs --check`（不构建，只判；不可信则退出码 1）。

每个用例独立临时 homedir + 独立 mock 实例（端口自动分配，18080 起），
进程结束自动清理；runner 层面每个用例再套一层独立子进程，互不拖垮。

## 新增一个用例

1. 新建 `cases/t7-<名字>.mjs`（`t<数字>` 前缀保证排序稳定）；
2. 首行 `import './_selfrun.mjs';`（自执行引导）；
3. `export default defineCase('T7 描述', async () => { ... })`，
   断言用 `helpers.mjs` 的 `assert` / `assertEq` / `assertTranscriptInvariants`。

没有清单要登记：`run-tests.mjs` 与门控（`scripts/gate.d/40-e2e.mjs`）都按
目录**发现式**加载，保存即生效。

## 场景编排（mock-llm）

| 字段 | 语义 |
|---|---|
| `match` | 对最后一条 user 消息做子串匹配 |
| `afterTool` | 只匹配「工具结果回灌轮」（最后一条消息 role=tool）；**不带它**的场景只匹配用户轮，防止工具循环 |
| `once` | 场景只消费一次（适合「第 N 轮才调工具」的编排） |
| `chunks` + `chunkDelayMs` | 正文分片逐帧吐出（流式逼真） |
| `toolCalls` | 先吐 `tool_calls` delta（参数分两片流式），再吐正文 |
| `reasoning` | 正文前输出 `reasoning_content` SSE 分片（驱动 reasoning 节点） |
| `status` ≥ 400 | HTTP 故障注入 |

## 用例与不变量

| 用例 | 文件 | 验证 |
|---|---|---|
| T1 | `t1-text-stream` | SSE 分片 → stdout 拼接；转写落盘（turn/text 节点、status=completed）；请求含内置工具清单 |
| T2 | `t2-mcp-tool-roundtrip` | MCP stdio 工具回路：tool_calls → mock-mcp 执行（记录核对）→ 结果回灌 → 收尾；转写成对 |
| T3 | `t3-vdfs-write` | 内置 `vdfs_write`：文件真实写入工作目录 |
| T4 | `t4-multi-turn` | 多轮会话：第二轮请求携带第一轮历史 |
| T5 | `t5-llm-http-error` | LLM HTTP 500：失败收敛，存储中无停在 streaming/pending 的节点 |
| T6 | `t6-mcp-tool-error` | MCP 工具 JSON-RPC 错误：错误结果回灌，会话照常收敛（auto 模式） |
| T7 | `t7-abort-convergence` | 中止收敛：REPL 长驻 + gateway HTTP invoke 中止 → `vdfs/stat` 读 `outcome=aborted`，节点终态化 |
| T8 | `t8-compression` | 压缩水位触发：`max_context_tokens` 调小 → 摘要请求 → 快照落库 → 历史归并 |
| T9 | `t9-ws-stream` | gateway WS 实时面（`event_bus` 的 `vdfs` 频道 + `vdfs/watch` 登记）：变更非空（两步订阅缺一不可）、首帧全量 + 窄 `delta`、增量拼接 == 完整正文、同一节点 `seq` 逐帧相同、会话收尾帧到达晚于全部消息帧 |
| T10 | `t10-node-protocol` | 节点状态机全景：Turn/Reasoning/Text/ToolCall 三态协议（`delta` 必先有身份帧、终态收敛、无孤儿）；先后按**到达序**判（单一 FIFO） |
| T11 | `t11-compression-node` | 压缩节点协议：`msg_type=compression` 消息节点、两态流（终态带结果正文）、位置契约（早于所属 Turn）、失败 `failure_kind`、重写 `removed` + 快照收敛 |
| T12 | `t12-content-type` | LLM POST 必须带 `Content-Type: application/json`（`&[u8]` + `.body()` 不会自动补） |
| T13 | `t13-session-options` | 会话选项 schema 化：定义挂 `new_types[].schema` 与清单项 `schema`（逐字节相同）、值随节点 `metadata` 回读、`vdfs/write` 浅合并落库、`stat` 不带定义、重启后仍在 |
| T14 | `t14-no-redundant-vdfs` | **会话期间零回读**：一轮会话的路由留痕里不得出现 `vdfs/stat` / `vdfs/read` / `vdfs/list`——变更必须自带载荷（节点视图 / 正文 / 目录清单） |
| T15 | `t15-subagent-inbox` | 子智能体空间**自驱动**：往 `<根>/agent/<id>/session/<sid>/inbox` 写即发消息（无调用方）；空间自己消费成完会话（FIFO、忙则排队、排队中可取消）；子空间带自己的 `AGENTS.md` 与自己的模型服务 |

每个用例共享的不变量断言（`assertTranscriptInvariants`）：

- `seq` 严格递增、消息 id 唯一；
- 每个 `tool_call` 必有 `role=tool` 结果子节点（「有请求必有响应」）；
- 失败路径不得留非终态节点（无「永远运行中」）。

## 门控接入

e2e 是 `scripts/gate.mjs` 的一个阶段（`scripts/gate.d/40-e2e.mjs`）：

```bash
node scripts/gate.mjs --only=e2e      # 只跑 e2e 阶段（先按源码指纹确保二进制最新）
node scripts/gate.mjs                 # 全量门禁（e2e 在静态审计之后）
```

用例清单不在门控里维护——与 runner 同一套发现逻辑，新增用例文件自动纳入。

## 提交

提交相关工作（门禁 → 生成合规消息 → git commit → 清理临时文件）收在一条命令里：

```bash
node scripts/commit.mjs               # 交互式
node scripts/commit.mjs --dry-run --yes --type=feat --scope=e2e --title="..."   # 预览消息
node scripts/commit.mjs --no-gate     # 紧急热修跳过门禁（自担风险）
```

## 首个战果

T2 首轮即抓到一个单测覆盖不到的真 bug：
`mcp/stdio.rs::decode_line` 在 Windows 上先按 GBK 解码，导致规范 UTF-8
MCP server 的全部非 ASCII 内容**静默乱码**后回灌给模型（GBK 几乎能无错
解码任何 UTF-8 字节，「失败回退」的注释是虚构的）。已改为先严格校验
UTF-8、仅非法时回退 GBK（见该函数文档）。
