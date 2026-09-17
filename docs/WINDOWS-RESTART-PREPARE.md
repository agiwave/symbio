# Windows 外部构建准备（prepare-only）

入口：`scripts/windows-restart-prepare.mjs`。Windows、Node.js 20+、PowerShell、Cargo/MSVC 工具链；无新增 npm 依赖。

**这不是自动重启器。** 当前仅交付一次性 prepare：未验证可靠的停机、应用就绪、数据目录交接及中断回合恢复协议，因此不实现 stop/start/switch。没有任何杀进程、启动候选应用、计划任务、持久服务、提权、权限绕过或 heartbeat 设置操作。不要将 `ready.json` 理解为可以直接替换生产实例的许可。

## 已读构建入口与范围

- `cli/Cargo.toml`：独立 Cargo workspace，二进制 `symbio-cli`。本脚本支持这种直接 Cargo 构建。
- `cli/.cargo/config.toml` 是本机被忽略的配置，可能把 target 指向核心缓存。本脚本用显式 `--target-dir` 和 `CARGO_TARGET_DIR` 覆盖它，在每次新建的随机子目录里编译，**不复用或覆盖运行中的 exe**。
- `tauri/package.json`、`tauri/src-tauri/tauri.conf.json`：桌面应用通过 Tauri 构建，`beforeBuildCommand` 为 `npm run build`，前端产物为 `../dist`。本脚本**不编排前端、Tauri 打包/安装，也不宣称交付桌面发行版**。
- 固定使用 `cargo build --locked --offline --release --manifest-path … --bin … --target … --target-dir …`；不联网补依赖，不传任意 shell 命令。必须已有 Cargo.lock 和依赖缓存。隔离目录意味着 C/C++ 原生依赖也可能全部重编；缺工具链/离线资源时直接失败，旧实例不受切换操作影响。
- 构建脚本及 Cargo 配置必须可信：Cargo build.rs 本身可以执行代码，target 隔离不是安全沙箱，不能限制恶意构建脚本的写入或进程操作。

## 配置与调用

在**独立的、已有正常构建环境的终端**运行。选择目标 PID 必须由用户完成，不按名称枚举/猜测宿主。不需要管理员权限；读不到进程路径/创建时间就失败，不提升权限。

用 PowerShell 只读查询**你明确选定的 PID**（12345 仅为占位符）：

```powershell
$p = Get-Process -Id 12345 -ErrorAction Stop
@{ pid=$p.Id; exe=$p.Path; startTimeUtcTicks=$p.StartTime.ToUniversalTime().Ticks.ToString() } | ConvertTo-Json
```

将输出填入配置。创建时间必须保留为字符串，防止 JSON 数值精度丢失。`cwd` 和 `args` 必须依据原启动方式由用户明确填写，包括无参数时的 `[]`；脚本**不能验证它们就是运行进程的原始 cwd/参数**，不猜测，也不据此启动进程。

先手动创建专用 staging 根目录。以下 JSON 路径、PID、创建时间均为示例，必须改为真实值：

```json
{
  "target": {
    "pid": 12345,
    "exe": "D:/Applications/symbio/symbio-cli.exe",
    "startTimeUtcTicks": "638900000000000000",
    "cwd": "D:/Bing/symbio",
    "args": []
  },
  "cargoExe": "C:/Users/YOU/.cargo/bin/cargo.exe",
  "buildCwd": "D:/Bing/symbio/cli",
  "manifest": "D:/Bing/symbio/cli/Cargo.toml",
  "bin": "symbio-cli",
  "triple": "x86_64-pc-windows-msvc",
  "stageRoot": "D:/SymbioPrepare",
  "cancelFile": "D:/SymbioPrepare/cancel.flag",
  "timeoutSeconds": 1800
}
```

所有路径必须绝对；已有文件/目录在入口检查。`triple` 支持 x86_64/aarch64/i686 的 `pc-windows-msvc`；未知字段拒绝。配置及交接记录会包含参数，可能含敏感数据，请保存在受控本地目录中，不要提交到仓库。

```powershell
# 默认 dry-run：只读配置/检查路径并向 stdout 打印计划。
# 不启动子进程，不创建目录/日志文件，不查询运行实例，不构建。
node D:/Bing/symbio/scripts/windows-restart-prepare.mjs --config D:/SymbioPrepare/config.json

# 显式执行：仍然只 prepare，绝不停止/启动应用。
node D:/Bing/symbio/scripts/windows-restart-prepare.mjs --config D:/SymbioPrepare/config.json --execute
```

退出码：0 为 dry-run 或 prepare 成功；1 为配置、身份、取消、构建、校验或超时失败。执行打印 `Stage:`，其中保留：

- `supervisor.log`：时间、构建命令、成功/失败原因。
- `build.stdout.log` / `build.stderr.log`：构建输出。
- `target/`：独立构建树（失败也保留，便于排查）。
- `ready.json`：仅成功后生成，含候选路径、大小、SHA-256、目标身份和使用限制。

入口参数错误、入口路径校验失败、执行前已取消等尚未创建 stage 的情况只输出到终端，不生成日志文件。dry-run 要保留记录可自行重定向 stdout。

## 安全边界、取消与超时

1. 执行前及构建/静态校验后核对 **PID + exe 规范路径 + UTC 创建时间 ticks**，任一不同或不可访问则失败。这是 prepare 时的观测，**不是之后人工切换时仍然有效的身份保证**。
2. 静态校验只检查 DOS/PE 签名、目标架构、非 DLL 可执行标志，并记录 SHA-256。**不代表加载成功、DLL 齐全、UI/服务就绪或数据兼容**；不启动候选程序作健康检查，避免它触发会话/heartbeat。
3. 在另一个终端创建取消文件（脚本只检查存在性，不读取内容）：
   ```powershell
   New-Item -ItemType File D:/SymbioPrepare/cancel.flag -Force
   ```
   执行入口、身份核验后、构建轮询（约 100ms）、构建后及发布记录前检查。文件应保持存在直到本次执行结束；删除取消文件后才能明确重新执行。没有旧实例终止步骤，所以不存在遗漏“终止前取消检查”的自动切换窗口。
4. `timeoutSeconds`（1..7200）是一次 prepare 的等待预算：身份查询单次最多 10 秒，构建使用剩余预算；静态同步文件检查前后检查预算，不是操作系统级实时硬限时。
5. **取消/超时不杀任何进程，包括构建进程。** 监督停止等待、以失败退出，不生成 ready；Cargo/编译器子进程可能继续写隔离目录。必须等用户确认它们退出后再清理该目录，避免重复启动昂贵构建。不能将此机制描述为强制终止构建树的超时器。
6. 成功后才创建的取消标记不会撤销已有 `ready.json`，它也不会引发自动切换。任何以后人工停旧实例之前，必须重新确认取消标记、进程完整身份、候选哈希和用户许可；本次 prepare 的检查不能替代这些检查。
7. 不循环、不重试、不修改宿主/heartbeat、不自动清理、不提供自动回滚或接管。用户应另行确认停机窗口、备份、运行依赖及数据目录独占访问；不要让新旧应用同时写同一个数据目录。

## heartbeat 是否自动恢复？

代码依据：

- `symbio/src/plugins/session/heartbeat.rs`：配置在 `Session.metadata.heartbeat`，循环扫描已启用且空闲的会话，15 秒扫描，带错峰与每 tick 上限。
- `symbio/src/plugins/session/plugin.rs`：有 Tokio runtime 时启动 heartbeat 循环；启动清理会把上次遗留的 `Streaming` 消息标成 `Failed`（“会话因重启中断”），`WaitingUserAction` 保留供用户 resume。
- `cli/src/main.rs`：`--heartbeat` 模式保持 CLI 存活来承载后端调度器，并非恢复任意已中断调用栈。

结论：**如果新实例正确加载同一持久会话存储、原任务仍 enabled、运行时/模型配置可用且进程持续存活，调度循环会重新启动，已有 heartbeat 可能继续自动触发。不能保证原回合自动续接，更不保证恰好一次。** 这不是“完全不自动恢复”，也不是“必然无损恢复”。

本 prepare 没有启动新实例，因此未实测应用重启后的 heartbeat；需要用户确认数据目录、enabled/prompt/interval、审批等待状态与实际触发结果。已有启用任务在新实例启动后可能很快触发；上线前应由用户审查。不应为了验证恢复而擅自开启心跳。

## 实测与复现

```powershell
node --check scripts/windows-restart-prepare.mjs
node --check scripts/windows-restart-prepare.test.mjs
node --test scripts/windows-restart-prepare.test.mjs
```

测试仅在 Windows 执行，其他平台显式 skip。需要 PATH 上可用的 cargo.exe 及 x86_64 MSVC 链接环境；可用 `PREPARE_TEST_CARGO` 指定 Cargo 路径。无第三方测试依赖。

测试创建带空格路径的临时零依赖 Cargo 小项目，离线真实编译；目标 PID 显式指向测试自身 Node 进程，而非 Symbio。覆盖默认 dry-run 无副作用、必填/未知字段、执行前取消、创建时间不匹配、真实隔离构建成功、真实 Rust 编译失败、损坏 PE/架构/DLL 校验失败、超时与运行中取消。超时/取消用短生命周期 Node 子进程验证其未被杀且能自然退出；关键路径检查目标身份和 exe 字节不变。

本次执行：`node --test scripts/windows-restart-prepare.test.mjs` 退出 0，10 tests / 10 pass / 0 fail / 0 skipped（包含父测试）。首次证据目录：`C:/Temp/symbio prepare test qVe3y7`，含配置、构建日志及候选；为审计保留，不自动删除。

**未实测完整 Symbio 构建、桌面打包、切换、应用就绪或 heartbeat 重启恢复。** 静态校验失败测试为校验器测试；超时/运行中取消测试为子进程等待器测试，并非强行挂起真实 Cargo。测试未关闭或重启当前 Symbio 实例，也未开启 heartbeat。
