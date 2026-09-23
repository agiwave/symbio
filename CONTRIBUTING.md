# Contributing to Symbio

感谢你考虑为 Symbio 贡献代码！本项目采用**分形插件架构 (Fractal Plugin Architecture)**，
整套能力由一个 Rust 核心库 + Tauri (Vue 3) 前端 + 多个可选插件协同完成。
为了保持代码质量与可维护性，请先阅读以下约定。

---

## 1. 开发环境

| 工具 | 版本 | 说明 |
|---|---|---|
| Rust | **1.93.1（锁定）** | 由 `symbio/rust-toolchain.toml` 自动激活；CI 与本地必须同版，否则 `cargo fmt --check` 会因格式规则漂移而失败 |
| Node.js | ≥ 18 | 前端构建 |
| Tauri CLI | 2.x | `cargo install tauri-cli --version "^2.0"` |
| 平台 | Windows / macOS / Linux | Tauri 三平台均已配置 CI |
| **系统库（仅 Linux）** | OpenSSL 开发包 + `pkg-config` | HTTP 出口的 TLS 走**平台原生栈**（[ADR-013](./docs/DECISIONS.md)）：Windows = SChannel、macOS = Security.framework、**Linux = 系统 OpenSSL** ⇒ Debian/Ubuntu 装 `libssl-dev pkg-config`，RHEL/Fedora 装 `openssl-devel` |

可选：使用 [rustup](https://rustup.rs/) 安装 Rust — `rust-toolchain.toml` 会自动激活对应的 channel 与组件。

> **为什么只有 Linux 需要额外系统库**：TLS 后端是平台原生栈——Windows 用 SChannel、macOS 用
> Security.framework，两者都是**纯 Rust FFI 绑定**（无 C 源、无需额外系统包）；只有 Linux 落到
> 系统 OpenSSL，因此需要上面的开发包。Windows/macOS **无需 C/C++ 编译器**：最后一条 C 编译链
> `onig_sys` 已随 `fastembed` 废弃、嵌入推理切到 `tract-onnx`（纯 Rust，[ADR-014](./docs/DECISIONS.md)）
> 退出依赖树；Linux 侧同理只剩编译 Rust 本身。

---

## 2. 仓库结构

```
symbio/
├── symbio/              # Rust 核心库（crate 根：cargo 命令在此目录跑）
│   ├── src/
│   │   ├── symbio_core/ # 框架层：Plugin trait / 路由 / Schema
│   │   ├── plugins/     # 业务插件（各含 README.md）
│   │   ├── providers/   # 基础设施（嵌入、文件存储）
│   │   ├── init.rs      # 根插件装配入口
│   │   └── lib.rs
│   ├── rust-toolchain.toml   # 锁定 Rust 版本
│   └── Cargo.toml
├── tauri/               # Vue 3 桌面端
│   ├── src/             # 前端代码
│   └── src-tauri/       # 仅 3 个 Tauri command 的薄适配层
├── cli/                 # 纯 Rust CLI 前端（进程内直连插件树）
├── docs/                # 系统级文档（历史归档在 docs/archive/）
├── scripts/             # 门禁与生成脚本（gate.mjs / gen-current-facts.mjs …）
└── rustfmt.toml · clippy.toml   # 位于仓库根
```

> 代码规模数字（文件数 / 行数）随迭代变化，不在此锁死——见 [docs/CURRENT.md](./docs/CURRENT.md) §5（自动生成）。

---

## 3. 提交前检查

**一条命令跑完全部**（后端含 `cli/` → 前端 → 静态审计 → MSRV → 事实文件）：

```bash
node scripts/gate.mjs                 # 全量
node scripts/gate.mjs --only=frontend # 单阶段（也有 --skip=）
node scripts/gate.mjs --only=msrv     # 用 rust-version 声明的最低工具链真跑一次 check
node scripts/gate.mjs --ci            # 对齐 CI（cargo test --workspace）
```

覆盖：Rust 编译 / 单测 / clippy / rustfmt、TypeScript 类型检查、vitest、静态审计、
**MSRV 实编译校验**、事实文件生成。完整输出落 `.workbuddy-ai/gate-logs/`；通过数低于基线
会报错、高于基线提示更新 `BASELINE`。

**格式化与事实文件由门禁自己做完，不用你动手。** `cargo fmt` 与 `gen-current-facts` 是
确定性的机械变换（函数，不是判断），所以门禁**直接执行**它们：命令跑成功即通过，命令本身
报错才不通过；本地还会把改写的文件**当场暂存**，使修复与本次提交是同一份内容。在 CI 里
（`--ci`）门禁不能提交，因此「执行后仍有差异」只能报红——那是唯一能保住不变量的信号，
出现时在本地跑一次门禁再提交即可。

⚠️ **上面这条「CI 判红」完全依赖 `--ci`**，而漏写 `--ci` 不会报错、不会变慢、本地也复现不出来
——它只会把这一步退化成「修复完静默放过漂移」，即一个只亮绿灯的检查项。所以 CI 里凡是
跑到了自动执行阶段（`backend` 的 fmt、`facts`）的调用**必须**带 `--ci`；回归测试
`scripts/gate.d/_shared.test.mjs` 会断言这一点。

**e2e 的被测二进制同理：不用你记得重建，也不靠文档提醒。** 判据在
`scripts/cli-binary.mjs`（门禁与 e2e 共用的唯一真相），是**内容指纹**而非「文件在不在」：
`cli/src` + `symbio/src` 的全部源码与两个 crate 的清单算一个 sha256，构建成功后写成构建戳；
使用前比对，不一致就先 `cargo build --release`（cargo 自己判增量）再写戳，`--check` 只判不建。
「文件在就算新鲜」这个判据曾经存在过，代价是一份过期 exe 被一直用下去，e2e 报出**与眼前源码
直接矛盾**的断言失败（源码里明明有的字段，运行时是 `undefined`），排查方向被引到源码上。

**壳（Tauri）那一侧同理，而且更贵。** 它不会报错，只会让**日志**看起来来自当前源码：于是
「日志里有一条源码中不存在的行」会被当成「代码没接上」，去读一遍代码。机制在
`scripts/tauri-binary.mjs`，指纹覆盖壳自身的源码 / 权限集 / 清单 + **整棵 `symbio/src`
插件树**（壳把它编译进去，插件日志正是从那里来的）。`npm run tauri dev|build` 会在构建**前**
把指纹写成构建戳（`before*Command` → `npm run stamp:dev|release`），壳启动时读**自己旁边**
那个戳并打进日志首行。于是任何一段日志自带「我是谁」，与 `node scripts/tauri-binary.mjs
--print` 一比即知是否同源；`--check` 则回答「本机产物对不对得上当前源码」。
⚠️ 戳是**构建前**写的声明，所以判据里还有一条「产物不得比戳更旧」——构建失败时旧产物会留在
原处而戳已指向新输入，那正是「把过期产物当最新」。配套地，输入未变时戳**不重写**，否则每次
「什么都不用重建」的构建都会变成一次假警报。

MSRV 阶段会换编译器（`RUSTUP_TOOLCHAIN` 覆盖 `rust-toolchain.toml`）并写独立 target
（`.workbuddy-ai/msrv-target/`），所以**不会**动日常构建缓存；本机没装该工具链时跳过并提示
（`rustup toolchain install 1.91.0`），CI 的 `msrv-check` job 装好后一定会真跑。

⚠️ **检查项的权威清单只在 `scripts/gate.mjs`**——本文档、CI、记忆里都不另抄一份，
抄了必然漂移。要加检查就改 `gate.mjs`。

找不到某条机制 / 约定写在哪：`node scripts/doc-find.mjs <关键词>`（搜全仓 `*.md` 与源码
`//!` / `///`）。项目文档是下沉的，**知识只写一处**；发现缺文档就补那一处，不要把摘要抄到别处。

CI（[.github/workflows/ci.yml](./.github/workflows/ci.yml)）跑的是**同一个脚本**
（`--only=backend --ci --profile=<dev|release>` / `--only=frontend` / `--only=docs,facts --ci`
/ `--only=msrv`——最后一个由独立的 `msrv-check` job 跑，它会先装 1.91 工具链），
所以本地通过 ≈ CI 通过。注意后两个 `--ci` 不是可选项：`backend` 含 `cargo fmt`、
`docs,facts` 含事实文件生成，两者都是门禁**自动执行**的工作，少了 `--ci` 就没有判红手段。

提交信息规范由 `scripts/check-commit-msg.mjs` 判定，**两处都要接上**：

```bash
git config core.hooksPath scripts/git-hooks   # 本机：写提交时即时拦
```

CI 侧另有 `commit-msg-check` job，用 `--range` 把本次引入的提交逐个判一遍
（判据只有脚本里那一处，两边不重抄）。

> **只装本机钩子是不够的**：那是一次性的本机配置，谁 clone 下来忘了设就形同没有；
> 反过来只靠 CI 也不够——要等到推送才会红。2026-09-20 复核时发现两处**都没接上**（钩子
> 文件在、`CONTRIBUTING` 也写了命令，但 `core.hooksPath` 为空且 CI 从不跑它），
> 于是这条规范实际上一直靠"提交者记得手动跑一次"维持。

### 本机操作陷阱（都踩过）

- ⚠️ **cargo / git 一律不接管道**（`| tail` / `| head`）：输出到 EOF 才刷 ⇒ 看着像卡死；且管道**吞掉退出码** ⇒ 失败的命令看起来是成功的。要看长输出就用 `gate.mjs`（它落日志、只信退出码）。
- ⚠️ **`git commit -F /c/Temp/msg.txt` 会失败**（Windows git 不认 MSYS 路径）⇒ 写 `-F "C:/Temp/msg.txt"`。
- ⚠️⚠️ **批量改名绝不用 `git rm` / `git mv`**：它们写索引，被 SIGTERM 中断后留下 0 字节 `.git/index.lock` 并写坏索引 ⇒ 上百文件误报「已删除」。正解 = 纯文件系统改名 + 最后一次 `git add -A`。**恢复**：`rm -f .git/index.lock` → `git checkout -- .`（索引损坏时 `reset --hard` 不可靠）。单个文件用 `git mv` 无妨。
- ⚠️ **无备份绝不 `git checkout -- .` / 大范围删除**。回滚先 `stash push` 或 `git diff > /c/Temp/x.patch`。
- ⚠️ **rustfmt 只用 `cargo fmt`**，绝不裸跑 `rustfmt`（工具链锁定版本不同 ⇒ 格式漂移）。
- ⚠️ 改 `.github/workflows/*.yml` 后**先自检语法**：YAML 错了 CI 会直接不触发（而非报错），
  很容易误判成"没跑"。本机若无 `js-yaml`，可用 `python -c "import yaml; yaml.safe_load(open(...))"`
  （必要时在隔离 venv 里 `pip install pyyaml`）。
- ⚠️ **改 `.rs` 后必须保持 LF**（`.gitattributes` 锁定 `*.rs text eol=lf`，对齐
  `rustfmt.toml` 的 `newline_style = "Unix"`）。用脚本批量改 Rust 文件时尤其容易踩：
  写入时把 `\n` 转成了 CRLF，rustfmt 会**逐文件**报 `Incorrect newline style` 并以
  非零码退出，症状是「fmt 门禁突然挂了但代码没变」。
- ⚠️ vitest 4 的 `toBe(v, 'msg')` 只收 1 个参数（写两个参数静默失效）。
- ⚠️ **「棘轮」（单向基线）不止 `gate.mjs` 里那一处**，`scripts/` 下多个审计脚本各自带一个常量，
  且方向**不统一**，别记反：
  - `gate.d/_shared.mjs` 的 `BASELINE.rustTests` / `vitestFiles` / `vitestTests` 是**地板**——
    实际值**低于**基线直接**报红**（「有测试被删或失败」），**高于**基线只打黄字让你上调。
  - `test-layout-audit.mjs` 的 `INLINE_TEST_BASELINE` 是**天花板**——实际值**高于**基线**报红**
    （「新增内联测试不被接受」），**低于**基线只打黄字让你下调。
  两者共同点是：**漏了只打黄字，`gate.mjs` 仍然是绿的** ⇒ 「门禁通过」不等于收工。删代码、
  删文件、拆测试文件之后，请**逐个复跑** `scripts/*-audit.mjs` 并读它的警告；改常量时**在常量旁
  写明日期与原因**（`git log -S<常量名>` 能查到历次调整的判据）。
- ⚠️ **本仓库的提交会被自动推送到远端**（`origin` = GitHub）：`git commit` 之后
  `origin/main` 立即前移（reflog 记 `update by push`），推送方**不是** `scripts/commit.mjs`
  （它明确不 push），而是本机环境侧的同步。含义是**没有「先提交错了再 amend」的余地**——
  提交前必须真的确认内容无误，amend 只会再造一段分叉的远端历史。
- ⚠️ `.workbuddy-ai/` 被 gitignore：**不要提交、不要删除**。仓库另有 `.workbuddy/`（旧 harness 遗留）——以 `.workbuddy-ai/` 为准。

---

## 4. 代码规范（要点）

### 文档与注释：各写各的，不互相复述

**同一件事写两处，改一次就要动两处，且两处必然漂移。** 因此按「谁拥有这条事实」分工：

| 写什么 | 归属 |
|---|---|
| 读这个文件需要知道的**不变量 / 陷阱 / 为什么这么写** | 该文件的 `//!` / `///`（就近） |
| 模块**机制与配置面** | 该模块 `README.md` |
| **路由 / 错误码 / 配置项 / 协议形状** | `docs/` 对应参考页（见 [docs/README.md 职责边界表](./docs/README.md#文档职责边界一个事实只有一个-owner)） |
| **为什么这样设计**（含被否决的方案与理由） | `docs/DECISIONS.md` 一条 ADR |
| **改了什么、何时改的** | 提交信息 + `git log` |

- 注释**不要写**：系统架构综述、文档里已有的机制与表格、变更史（「曾经…已改为…」）。
  这些要么属文档、要么属 `git log`——写进注释只会在下次改动时漏掉一处。
- 注释**应该写**：为什么是这个常量 / 这个顺序、有什么坑、字段归谁消费（例：会话的工具结果
  字段约定）。
- 文档**不要抄**代码签名与字段表：要清单就读定义（或它的 `//!` 头注释）。手抄的表必然漂移
  ——本仓库已数次出现「文档列的文件早已不存在」。
- **过程文档必须归档**：一次性评审 / 迁移记录 / 已落地的实施方案一律 `git mv` 进
  `docs/archive/`（**模块目录同样适用**），由 `node scripts/doc-link-audit.mjs` 的 D-002 判定。

### Rust 侧

- 遵循 `rustfmt.toml` + `clippy.toml`，**不要**手动格式化后被 CI 反复退回。
- 异步上下文请使用 `tokio::sync::Mutex` / `tokio::sync::RwLock`；`std::sync::*` 只在 `spawn_blocking` 内部使用。
  - 审计脚本 `scripts/grep-audit.mjs` 中的 **S-002 规则**默认扫描全部插件（含测试代码）；这是邻近行启发式检查，不替代锁生命周期审查。
  - 已人工确认的 S-002 误报可在命中行末添加 `// grep-audit-allow S-002: 具体理由`，仅豁免该行，不应借此隐藏真实跨 await 持锁。回归测试 `node --test scripts/grep-audit.test.mjs` 已接入统一门禁。
- 日志：使用 `tracing`（`info!` / `warn!` / `error!` / `debug!`），**不要**用 `eprintln!` / `println!`。
  - 项目在 `tauri/src-tauri/src/main.rs` 已初始化 `tracing-subscriber`。
- 错误：实现 `thiserror` 派生 `PluginError` 变体，不要用 `String` 当错误类型。
- 新增能力：优先在合适的 `Plugin` 下添加子路径（`worker/xxx/yyy`），
  路由规则见 [docs/architecture/OVERVIEW.md](./docs/architecture/OVERVIEW.md)，
  请求全链路见 [docs/architecture/DATA_FLOW.md](./docs/architecture/DATA_FLOW.md)。

#### 测试文件布局（约定）

测试**独立成文件**，且与**被测试的实现文件同级**、同名加 `.test` 后缀：

```text
workdir.rs          实现
workdir.test.rs     它的测试（同级，不放子目录）
```

父文件末尾用 `#[path]` 指向同级测试文件：

```rust
#[cfg(test)]
#[path = "workdir.test.rs"]
mod tests;
```

要点：

- 布局由 `node scripts/test-layout-audit.mjs` 判定（已接入 `gate.mjs`），不靠人工记住。
- **一个实现文件对应一个测试文件**。不要写"一个测试文件同时测几个实现文件"，
  也不要写"几个测试文件测同一个实现文件"——测试跟着它测的那个实现走。
- **不要为测试文件单独建目录**。`X/tests.rs` 那种形态会让"模块"与"目录"两个概念
  混在一起；`#[path]` 相对声明它的文件解析，测试文件完全可以与实现同级。
- **例外**：模块文件是 `mod.rs` 的（如 `store/mod.rs`），测试放同级的 `tests.rs`。
- `#[path]` **不改变模块路径**（仍是 `X::tests`），故 `use super::*;` 语义与内联
  `mod tests { … }` 完全一致，拆分/搬移是纯文件操作，不涉及可见性调整。

> 存量代码里仍有大量内联 `#[cfg(test)] mod tests { … }`。**新增代码请按上述约定**；
> 存量按"改到哪个文件就顺手拆哪个"渐进处理，不做全仓一次性搬移。

### TypeScript / Vue 侧

- 使用 `<script setup lang="ts">`，**不要**使用 Options API。
- 严禁 `any`（callPlugin 泛型默认已是 `unknown`，调用方需显式标注）。
- 日志：使用 `logger` from `@/utils/logger`，**不要**用 `console.*`（CI 会查）。
- 插件路径：所有 worker 路径必须从 `@/constants/pluginPaths` 导入，
  **不要**在 `services/` / `stores/` / `composables/` 中硬编码 `'worker/...'` 字符串。

---

## 5. 插件开发流程

新增一个业务插件的最短路径：

1. 在 `symbio/src/plugins/<name>/` 下创建 `mod.rs` + `plugin.rs`。
2. 在 `symbio/src/plugins/mod.rs` 加一行**私有** `mod <name>;`（`mod` 刻意私有——插件之间互不可见）。
3. 实现 [`Plugin`](./docs/architecture/PROTOCOLS.md) trait 的 `route()` 与 `traverse()`；在文件末尾用
   `submit_object_creator!(PLUGIN_<NAME>, <Name>Plugin::build, dyn Plugin)` 自注册。
   **无需改 `init.rs`**：`home` 构造 `worker`(Composite) 时按目录扫描子插件——这正是"零配置"。
4. 若插件**可配置**（要出现在设置页）：在 `home` 的 `ensure_defaults` 加插件名、在 `setting` 的
   `SETTING_SECTIONS` 加 `(id, 中文名)`、并在该插件自己的 `detail_definition()` 加返回 `config_definition(...)` 的分支。
5. 若插件要出现在前端：在 `tauri/src/constants/pluginPaths.ts` 添加路由常量，`tauri/src/services/` 下加对应 TS 客户端。
6. 文档按**唯一来源**更新：**新增路由只在 [ROUTES.md](./docs/reference/ROUTES.md) 登记**（模块 `README.md` 不抄路由表，只写机制并指向它）；配置项进 CONFIGURATION.md、错误码进 ERROR_CODES.md；插件 / 挂载点 / 工具变更后重跑 `node scripts/gen-current-facts.mjs`。

> **变更历史就是 `git log`，本仓库不再维护 `CHANGELOG.md`。**
> git 记录**就是**变更历史，且比手抄的一份文件更准确——不会漏、不会与代码漂移，
> 也不必维护第二份同样的信息。提交信息本身已足够结构化（见上文「提交规范」：
> `<type>(<scope>): <中文标题>` + 编号分节 + 「门禁：」段），且**与代码同一次提交**，
> 不可能漂移。要查「某次改动为什么」：`git log --grep=<词>` / `git log -S<符号>`；
> 要查「某条机制写在哪」：`node scripts/doc-find.mjs <词>`。

完整教程见 [docs/guides/PLUGIN_DEVELOPMENT.md](./docs/guides/PLUGIN_DEVELOPMENT.md)。

---

## 6. Pull Request 流程

1. **Fork** 仓库，创建分支（`feat/xxx` / `fix/xxx` / `docs/xxx`）。
2. 提交信息推荐格式：
   ```
   <scope>: <summary>
   
   <details>
   ```
   scope 例：`agent` / `tauri/services` / `docs`。
3. 推送后通过 PR 提交，CI 必须全绿。
4. 至少 1 位 maintainer 审阅通过后可合入。
5. 涉及架构变更的 PR：决策本身写进 [DECISIONS.md](./docs/DECISIONS.md)（一条 ADR），
   「改了什么 / 为什么」写在提交信息里。**不另维护变更日志**（见上文第 5 节的说明框）。

---

## 7. 报告 Bug

请使用 GitHub Issues，并包含：

- 复现步骤（最小可复现 demo）
- 实际行为 vs 预期行为
- 平台（Windows / macOS / Linux）+ Rust / Node 版本
- 关键日志（启用 `RUST_LOG=debug` 或 `VITE_LOG_LEVEL=debug`）

---

## 8. 社区准则

参见 [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)。

---

## 9. 许可证

提交即表示你同意本项目以 [MIT License](./LICENSE) 发布你的贡献。
