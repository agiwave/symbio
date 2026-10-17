---
kind: guide
id: GU-003-building
title: 构建
status: active
created: 2026-09-11
---

# 构建

> **一句话**：不要直接 `cargo build`。用 `node scripts/build-cli.mjs`，由脚本把 C 工具链喂给
> cc-rs，并把路径分隔符归一化，保证与 `symbio/` 原生构建的环境逐字节一致。

```bash
node scripts/build-cli.mjs            # cargo build --offline
node scripts/build-cli.mjs --check    # 只做类型检查（更快）
node scripts/build-cli.mjs -- --release   # `--` 之后原样透传给 cargo
cd cli && cargo test                  # 参数解析单测（4 个用例）
```

产物：`../target/debug/symbio-cli.exe`（相对 `cli/`）。**没有第二个 target 目录**——`cli` 是仓库根
workspace 的成员（根 `Cargo.toml` 的 `members`），cargo 把所有成员的产物写进同一个 `<仓库根>/target/`，
依赖缓存因此天然共享；构建成功后脚本把这一行原样打印出来。

---

## 为什么不能裸 `cargo build`：三条实测结论

### ① C 编译器必须显式注入，否则构建脚本静默挂死

`symbio` 依赖树里 `aws-lc-sys`（reqwest→rustls 的加密后端）、`ring`、`onig_sys`、
`libsqlite3-sys` 等都要 C 编译器，经 cc-rs 探测。在没有 VS 开发环境的终端里探测不到 `cl.exe`，
构建脚本**既不报错也不退出**（进程活着、CPU≈0、无子进程），看起来像 cargo 死锁，实际是在等编译器。

脚本只注入 cc-rs 与链接器真正需要的四个变量（`CC` / `CXX` / `INCLUDE` / `LIB`），**刻意不动
`PATH`** —— 保持调用方环境原样，避免影响其它工具链探测。

**关键**：`aws-lc-sys` 0.44 自带 `builder/prebuilt-nasm`（预编译 NASM 对象），所以**既不需要
cmake，也不需要安装 nasm**，只要有 `cl.exe` + `INCLUDE`/`LIB` 即可。链接用的 rustup 自带
`rust-lld.exe` 也读 `LIB` 找 Windows SDK 导入库（`LIB` 不进 cc-rs 指纹，设了不会触发 C 重编）。

### ② `INCLUDE`/`LIB` 必须归一化成 `/`，否则每次全量重编

cc-rs 把这两个变量记进构建指纹（`rerun-if-env-changed`），值一变就判定整棵 C 依赖树过期。而 Git
Bash 的 MSYS 在 spawn 原生 `.exe` 时会重写 `;` 分隔的路径列表，把反斜杠形式（`…\Apps\…`）改写成
正斜杠形式（`…/Apps/…`）。
于是「一次用脚本构建、一次用 shell 构建」得到两种字符串交替 ⇒ `aws-lc-sys` 反复全量重编（每次约 9
分钟）。

脚本统一输出 `/` 分隔形式（`sep = s => s.replace(/\\/g,'/')`），与 MSYS 改写后的产物一致 ⇒ 两种
入口写下的指纹相同。

**排查手法**：
```bash
CARGO_LOG=cargo::core::compiler::fingerprint=info cargo build 2>&1 | grep -i dirty
# 会直接点名：dirty: EnvVarChanged { name: "LIB", old_value: "<盘符>:\\..." (反斜杠), new_value: "<盘符>:/..." (正斜杠) }
```

### ③ target 目录只有一个：仓库根的 `target/`

`cli` 是仓库根 workspace 的成员（根 `Cargo.toml` 的 `members`），cargo 因此把所有成员的产物
写进**同一个** `<仓库根>/target/`——缓存天然共享，不需要任何 target-dir 配置。

不要为本机另设 `build.target-dir`，尤其不要设成相对路径：cargo **不归一化**它，
`../symbio/target` 会原样拼成 `<仓库根>\cli\../symbio\target`，而这个字符串进入构建脚本的
`OUT_DIR` / `CARGO_TARGET_DIR` 环境后，与另一路构建用的绝对写法不等价，于是 `aws-lc-sys` /
`ring` / `onig_sys` / `libsqlite3-sys` / `ort-sys` 全部被判定过期强制重编 → 在缺 C 工具链的环境挂死。
另需注意 `.cargo*/` 在本仓是 **gitignore** 的：本机那份覆盖在仓库里看不见，而它一存在，产物就
离开规范位置——`scripts/cli-binary.mjs` 按多个候选位置探测，正是为了不把「找不到」当成「缺失」。

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

`scripts/repair-target-cache.mjs` 以构建日志为唯一证据修：跑一次
`cargo build --offline --keep-going`，从 `E0514` 的 note 行解析出被判定为异版本的 crate，删掉对应的
`.fingerprint/<pkg>-<hash>/`（缺失指纹 ⇒ cargo 必然重编），循环直到构建成功。默认 dry-run，加
`--fix` 才动文件。

含 C/asm 构建脚本的 crate（`aws-lc-sys` / `ring` / `ort` / `libsqlite3-sys` …）在**禁区名单**里，
**绝不删** —— 它们在缺 C 工具链的环境里重跑会挂死。若禁区 crate 出现在 `E0514`，先按上面 ① 设好
`CC`/`INCLUDE`/`LIB` 再重跑。

```bash
node scripts/build-cli.mjs --check                    # 先看当前是否干净
node scripts/repair-target-cache.mjs --fix
```

> 该修复思路已沉淀为通用 skill `cargo-target-cache-repair`（`~/.workbuddy/skills/`），可跨项目复用。
