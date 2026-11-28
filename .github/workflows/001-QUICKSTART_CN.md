---
kind: guide
id: GU-001-QUICKSTART_CN
title: 🚀 自动构建指南
status: active
created: 2026-07-20
---

# 🚀 自动构建指南

桌面交付物是「CLI 二进制 + 系统浏览器」的轻量启动器，没有 WebView / 安装包链。
推送标签后，GitHub Actions 会在 Windows / macOS / Linux 上构建两个 Rust 二进制
（`symbio-cli` 后端 + 内嵌 UI、`symbio-desktop` 启动器），打成便携压缩包并创建 Release 草稿。

## 快速开始（5 分钟）

### 1️⃣ 更新版本号

版本号以根 `Cargo.toml` 为准（`release.yml` 的 `check-version` 会把 tag 与它比对）：

```toml
# Cargo.toml（根 package / workspace.package）
version = "0.1.14"
```

### 2️⃣ 提交并推送标签

```bash
git add .
git commit -m "chore: bump version to 0.1.14"
git tag v0.1.14
git push origin v0.1.14
```

### 3️⃣ 等待自动构建

推送标签后，GitHub Actions 会自动：

- ✅ 在 Windows/macOS/Linux 上并行构建
- ✅ 生成各平台便携压缩包（内含 CLI + 启动器两个二进制）
- ✅ 创建 GitHub Release 草稿

约 20-40 分钟后，在 **GitHub → Releases** 查看构建产物！

---

## 📦 构建产物

每个压缩包内含两个二进制：`symbio-cli`（后端 + 二进制内嵌 UI）与
`symbio-desktop`（轻量启动器）。运行 `symbio-desktop` 即用系统默认浏览器打开本机 UI。

| 平台 | 格式 | 说明 |
|------|------|------|
| **Windows x64** | `.zip` | 便携包，含 `symbio-cli.exe` + `symbio-desktop.exe` |
| **macOS Intel** | `.tar.gz` | 便携包（无签名，需手动信任） |
| **macOS Apple Silicon** | `.tar.gz` | 便携包（无签名，需手动信任） |
| **Linux x64** | `.tar.gz` | 便携包，含两个二进制 |

---

## ⚙️ 可选配置

### macOS 代码签名

发布的是便携压缩包，不是 `.app` / `.dmg`，因此**不配置 macOS 签名也能直接运行**
（Gatekeeper 对命令行二进制无 `.app` 签名要求）。密钥仅用于未来打包 `.app` 时：

| 密钥 | 说明 |
|------|------|
| `APPLE_CERTIFICATE` | .p12 证书的 Base64 |
| `APPLE_CERTIFICATE_PASSWORD` | 证书密码 |
| `APPLE_SIGNING_IDENTITY` | 签名身份 |
| `APPLE_ID` | Apple ID 邮箱 |
| `APPLE_PASSWORD` | 应用专用密码 |
| `APPLE_TEAM_ID` | 团队 ID |

详细配置指南: [002-SECRETS_SETUP.md](002-SECRETS_SETUP.md "GU-002-SECRETS_SETUP")

---

## 🔧 手动触发构建

1. 进入 GitHub → Actions
2. 选择 **Release** 工作流
3. 点击 **Run workflow**
4. 勾选 "创建 GitHub Release"
5. 点击运行

---

## 📖 完整文档

- [密钥配置指南](002-SECRETS_SETUP.md "GU-002-SECRETS_SETUP")
- [工作流配置](release.yml)
- [配置脚本](setup-secrets.ps1)

---

## ❓ 常见问题

**Q: 需要配置密钥才能构建吗？**

A: 不需要！直接推送标签即可构建，无需代码签名或自动更新签名。

**Q: 下载后怎么运行？**

A: 解压后运行 `symbio-desktop`（Windows 是 `symbio-desktop.exe`），它会在系统默认浏览器
打开本机 UI。也可以直接运行 `symbio-cli serve --browser`。

**Q: 离线能用吗？**

A: 能。前端 UI 已编译期内嵌进 CLI 二进制，前后端通讯走 `127.0.0.1` 回环，不需要外网。

**Q: 构建失败怎么办？**

A: 查看 Actions 页面日志，常见原因：依赖安装失败、编译错误。
