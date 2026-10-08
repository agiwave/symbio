# Windows 一次性重启监督（最小版）

入口：`scripts/windows-restart.mjs`；测试：`scripts/windows-restart.test.mjs`。Windows、Node.js 20+、Windows PowerShell、已有 Cargo/MSVC 离线构建环境，无 npm 新依赖、服务或计划任务。原 [prepare-only](WINDOWS-RESTART-PREPARE.md) 脚本及接口不变。

**只观察进程存活，不验证应用就绪、数据兼容、heartbeat 或会话续接。禁止把当前会话宿主作为目标。** 脚本拒绝自身及可查询到的启动祖先进程；后台入口将祖先 PID 传给监督子进程。此保护不是任意系统进程识别器，用户仍须明确选择独立实例。

## 使用

沿用 prepare 配置，另加：

```json
{
  "statusFile": "D:/SymbioPrepare/run-001/status.json",
  "aliveSeconds": 20
}
```

这两个字段需合并到完整的 prepare JSON 顶层，并非独立配置。`statusFile` 必填绝对路径，父目录须已存在；`aliveSeconds` 可选，默认 20，范围 1..600。所有原 prepare 必填项仍须提供（包括使用 ready 时），`target.cwd/args` 是用户明确提供的启动契约，不从进程猜测。每次运行使用专用 status 路径及受控 staging 目录，配置/日志可能包含敏感参数。

```powershell
# 默认仅打印计划，无子进程/构建/文件写入
node scripts/windows-restart.mjs --config D:/SymbioPrepare/restart.json

# 前台一次性监督：先 prepare 构建，再切换
node scripts/windows-restart.mjs --config D:/SymbioPrepare/restart.json --execute

# 使用已有交接记录，省略编译；也可在配置中设置 readyFile
node scripts/windows-restart.mjs --config D:/SymbioPrepare/restart.json --ready D:/SymbioPrepare/symbio-prepare-XXXX/ready.json --execute

# 独立后台监督；缺少 --execute 会拒绝
node scripts/windows-restart.mjs --config D:/SymbioPrepare/restart.json --ready D:/SymbioPrepare/symbio-prepare-XXXX/ready.json --execute --background

# 后续轮次查询
Get-Content -Raw D:/SymbioPrepare/run-001/status.json | ConvertFrom-Json
```

后台启动器返回 `supervisorPid/statusFile/launcherLogs` 后退出；仅证明 spawn 成功，不表示切换成功。监督子进程使用配置快照、detached 启动、stdin 忽略，stdout/stderr 重定向到 `launcherLogs`。candidate 和 rollback 同样独立启动、重定向日志，使用原 `target.cwd/args`，**直接运行 staging 中 candidate，不覆盖/搬移旧 exe**。监督完成后退出，不持续监控或自动重试。

## 切换与失败边界

1. 检查当前目标 PID、规范 exe 路径、UTC 创建时间 ticks。构建调用原 prepare；失败不进入 stop。`--ready` 同样检查记录身份、triple、PE、SHA-256、配置和原 ready 的 cancelFile。
2. 最后 stop gate 在 PowerShell 获取一个 `Process` 对象，先访问 `.Handle` 持有句柄，再在同一对象检查路径/创建时间、再次核对 candidate SHA-256 和取消标记，调用 `.Kill()`、`.WaitForExit()`、`.HasExited`。不按名称杀、不重新按 PID 获取另一个对象执行终止。
3. 确认退出后，再检查取消及 candidate hash，启动 candidate 并观察存活窗口。`Kill` 是强制停止，不是优雅停机协议；事先确认停机与数据风险。
4. 停止前失败：不启动 candidate 或 rollback。若终止操作已发生但 PowerShell 回执丢失/超时，记录 `failed-before-confirmed-stop`，**不猜测退出，也不贸然启动第二实例**；需人工核对。
5. 已确认旧实例退出后的失败：先等待/终止已启动 candidate，必须观察到退出才允许重新运行旧 exe；重新验证旧 exe hash，再以原 cwd/args 启动并观察同样窗口。取消不阻止安全回退。无法确认 candidate 退出或旧 exe hash 改变时禁止回退，记录 `recovery-failed`。
6. 只负责这些明确的进程，不管理进程树：会派生长期后台子进程、转交给已有实例后自行退出的程序不适用。无磁盘/数据库回滚；候选已写入的数据不撤销。无系统级沙箱、提权或权限绕过。
7. `timeoutSeconds` 是构建/切换/候选观察的检查预算，不是实时硬截止；退出等待有独立裕量，回退观察有独立窗口。沿用 prepare 的编译取消/超时语义：不杀编译进程，它们可能继续在隔离目录工作。
8. 可信本地配置与目录是前提：hash 是一致性检查而非签名，不能抵抗可同时篡改 ready 与二进制的攻击者；最后 hash 检查到启动仍有文件变化窗口。请保护目录 ACL，不并发替换 candidate。

## 状态与日志

`status.json` 通过同目录临时文件 + rename 更新，含 supervisor/target/candidate/rollback PID（适用时）、候选路径/hash、stage、时间、事件列表及错误。`applicationReady` 始终为 `false`。

- `verifying` → `building`（非 ready）→ `pre-switch` → `stopping-target` → `target-stopped` → `survival-window` → **`survived`**。
- 停止前/退出未确认：**`failed-before-confirmed-stop`**。
- 停止后失败：`recovering` → `candidate-exit-confirmed` → `rollback-window` → **`rolled-back`**；恢复失败为 **`recovery-failed`**。
- `survived`/`rolled-back` 只表示当时通过存活窗口，不能当成后续持续健康状态。

stage 保留 `supervisor.log`、`candidate.stdout/stderr.log`、`rollback.stdout/stderr.log`（适用时）及嵌套 prepare 构建证据。入口校验、祖先保护、锁失败可能发生在状态初始化之前：前台看 stderr，后台看 launcher stderr；此时旧 status 不能作为本次结果。

`statusFile.lock` 排斥使用同一路径的并发监督，不是全局目标锁；不要针对同一实例用不同状态路径并发运行。正常退出移除锁，异常中断保留。中断后须核对目标/candidate/rollback 身份、退出情况及监督 PID，再由人处理锁与重试，不自动接管。旧 exe、构建目录和日志均不自动清理。

前台退出码：0 dry-run/候选存活通过，1 失败（即使回退成功）。后台启动器退出 0 仅确认启动；最终结果查状态和日志。

## 实际测试

```powershell
node --test --test-reporter=spec scripts/windows-restart-prepare.test.mjs scripts/windows-restart.test.mjs
```

已在本机 Windows 实际执行：**22 tests / 22 pass / 0 fail / 0 skipped，退出码 0**（包含两组父测试）。restart 独立套件为 12 tests / 12 pass。保留最终证据：

- `C:/Temp/symbio restart test CWc3zd`
- `C:/Temp/symbio prepare test tsG9O9`

restart 测试用零依赖 Rust fixture 离线真实编译，目标为其独立进程，不是当前 Symbio。覆盖默认 dry-run、真实编译失败不杀、错误身份/hash/cancel 不杀（含 PowerShell 最后 gate）、真实停止/构建候选启动、cwd/含空格中文引号参数与日志、ready 跳过损坏源码构建、候选提前退出回退、存活窗口取消后终止候选再回退、后台启动器退出后监督子进程继续存活并完成、宿主祖先保护。测试最后清理专属 fixture 进程，保留文件证据。

哈希校验走 .NET SHA256（不依赖 PowerShell 的 `Get-FileHash`）。回退路径确保 child 的 kill error 不被误当成退出事件。

**没有进行真实 Symbio 切换、应用就绪或 heartbeat 恢复测试，未停止当前宿主。** 当前脚本还主动拒绝启动祖先作为目标，因此不是已完成的当前宿主自重启方案。

主会话最后复验：`node scripts/windows-restart.test.mjs`，12 tests / 12 pass / 0 fail / 0 skipped，退出码 0；证据 `C:/Temp/symbio restart test aEZwkr`。`node scripts/gen-current-facts.mjs --check` 通过。本轮没有重跑全仓门禁。

此前后台 CLI 构建日志 `.workbuddy-ai/restart/cli-build-1.93.1.log` 最终记录 `Finished dev profile`（3m 28s）；仅证明编译完成，没有启动该产物、真实应用切换或模型驱动心跳续接证据。后续应使用隔离 homedir 的真实实例验证，不能将上述 fixture 的进程存活结果代替应用验证。
