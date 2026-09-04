# Symbio 编译与排错指南

> **文档类型：How-to guide（操作指南）** — 本地编译、CLI 工具、依赖与排错。

* CI（`.github/workflows/`）包含两个工作流：
  1. 代码质量CI（`.github/workflows/ci.yml`）：
     - `cargo fmt --all -- --check`
     - `cargo clippy --workspace --all-targets -- -D warnings`
     - `cargo test --workspace`
     - `cargo build --release --manifest-path symbio/Cargo.toml`
     - `node scripts/grep-audit.mjs`（P16 已闭环）
     - `npx vue-tsc --noEmit`（TypeScript 类型检查）
     - `cargo audit`（安全审计）
  2. 发布构建CI（`.github/workflows/release.yml`）：
     - 预检 `quality-gate` 复跑 fmt/clippy/test/audit（防止 `workflow_dispatch` 绕过 ci.yml）
     - 构建 Windows / macOS (x64 + aarch64) / Linux Tauri 桌面应用
     - 创建 GitHub Release 并分发制品
* 日常开发应依赖代码质量CI快速反馈，发布时才触发发布CI。
* 发布产物为 `symbio` 二进制（E2E CLI）+ 必要的样例配置，无其他依赖。
* **Rust 工具链固定**：[`rust-toolchain.toml`](../symbio/rust-toolchain.toml) 位于 `symbio/` 目录
  （与 `Cargo.toml` 同级），pin 到 `channel = "stable"` 并启用 `rustfmt` / `clippy` 组件。
  在 `symbio/` 下执行任何 `cargo` 命令时 rustup 会自动按此配置下载工具链。