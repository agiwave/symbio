---
kind: guide
id: GU-003-building
title: 构建
status: active
created: 2026-09-11
---

# 构建

> **一句话**：用 `node crates/symbio-cli/scripts/build-cli.mjs`——它传 `--offline`，并在构建成功后
> 打印产物的绝对路径。

```bash
node crates/symbio-cli/scripts/build-cli.mjs # cargo build --offline
node crates/symbio-cli/scripts/build-cli.mjs --check # 只做类型检查（更快）
node crates/symbio-cli/scripts/build-cli.mjs -- --release # `--` 之后原样透传给 cargo
cargo test -p symbio-cli # 参数解析单测
```

产物：`../../target/debug/symbio-cli.exe`（相对 `crates/symbio-cli/`）。**没有第二个 target 目录**——`cli`
是仓库根 workspace 的成员（根 `Cargo.toml` 的 `members`），cargo 把所有成员的产物写进同一个
`<仓库根>/target/`，依赖缓存因此天然共享；构建成功后脚本把这一行原样打印出来。

---

## target 目录只有一个：仓库根的 `target/`

`cli` 是仓库根 workspace 的成员（根 `Cargo.toml` 的 `members`），cargo 因此把所有成员的产物
写进**同一个** `<仓库根>/target/`——缓存天然共享，不需要任何 target-dir 配置。

不要为本机另设 `build.target-dir`，尤其不要设成相对路径：cargo **不归一化**它，
`../target` 会原样拼成 `<仓库根>\crates\symbio-cli\..\target`，而这个字符串进入构建脚本的
`OUT_DIR` / `CARGO_TARGET_DIR` 环境后，与另一路构建用的绝对写法不等价，于是同一批 crate 会被
判定过期强制重编。

另需注意 `.cargo*/` 在本仓是 **gitignore** 的：本机那份覆盖在仓库里看不见，而它一存在，产物就
离开规范位置——`scripts/cli-binary.mjs` 只解析 `<仓库根>/target/<profile>/`，e2e 与门禁都从那里取
二进制；产物跑丢的形态是「构建成功但找不到二进制」，不是静默用旧产物。

---

## 其余必须对齐的两点

- **离线由脚本显式给**：`build-cli.mjs` 传 `--offline`，构建永不触网（不依赖 cargo 配置文件）。
 新增或升级依赖后要跑一次 `node scripts/cargo-offline-refresh.mjs` 把新 crate 拉进本地缓存，
 否则下一次离线构建直接报「缺少 crate」。
- **工具链只有一个来源**：根 `rust-toolchain.toml` 锁 `1.93.1`，成员目录不再各放一份。理由仍是
 `E0514: found crate X compiled by an incompatible version of rustc`——产物文件名**不含**编译器
 版本，两个 rustc 的产物混进同一个 target 目录时，cargo 会把异版本产物当成新鲜的直接复用。

---

## 万一 target 缓存已经被异版本编译器污染

`crates/symbio-cli/scripts/repair-target-cache.mjs` 以构建日志为唯一证据修：跑一次
`cargo build --offline --keep-going`，从 `E0514` 的 note 行解析出被判定为异版本的 crate，删掉对应的
`.fingerprint/<pkg>-<hash>/`（缺失指纹 ⇒ cargo 必然重编），循环直到构建成功。默认 dry-run，加
`--fix` 才动文件。

含 C/asm 构建脚本的 crate 在脚本的**禁区名单**（`BLOCKLIST`）里，**绝不删**——重跑它们的构建
脚本代价高且可能挂死。若禁区 crate 出现在 `E0514`，先让本机编译链可用再重跑。

```bash
node crates/symbio-cli/scripts/build-cli.mjs --check # 先看当前是否干净
node crates/symbio-cli/scripts/repair-target-cache.mjs --fix
```

> 该修复思路已沉淀为通用 skill `cargo-target-cache-repair`（`~/.workbuddy/skills/`），可跨项目复用。
