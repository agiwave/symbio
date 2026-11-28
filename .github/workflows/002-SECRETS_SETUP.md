---
lists: [github-secrets=owner]
kind: guide
id: GU-002-SECRETS_SETUP
title: GitHub Actions 密钥配置指南
status: active
created: 2026-07-20
---

# GitHub Actions 密钥配置指南

本文档说明 GitHub Actions 发布所需的密钥。发布产物是**便携压缩包**（Windows ZIP /
macOS·Linux tar.gz），内含 `symbio-cli` 与 `symbio-desktop` 两个二进制，不是安装包，
因此没有自动更新签名需求。

---

## 密钥清单

<!-- list:github-secrets owner -->

| 密钥名称 | 用途 | 必需 | 平台 |
|---------|------|------|------|
| `GITHUB_TOKEN` | GitHub 自动提供，用于创建 Release | ✅ 是 | 全部 |
| `APPLE_CERTIFICATE` | macOS 代码签名证书（可选） | ❌ 可选 | macOS |
| `APPLE_CERTIFICATE_PASSWORD` | 证书密码 | ❌ 可选 | macOS |
| `APPLE_SIGNING_IDENTITY` | Apple 签名身份 | ❌ 可选 | macOS |
| `APPLE_ID` | Apple ID（用于公证） | ❌ 可选 | macOS |
| `APPLE_PASSWORD` | Apple 应用专用密码 | ❌ 可选 | macOS |
| `APPLE_TEAM_ID` | Apple 开发者团队 ID | ❌ 可选 | macOS |

**说明**:
- ✅ **GITHUB_TOKEN** 由 GitHub 自动提供，无需手动配置
- 发布的是命令行二进制便携包，不配置 Apple 证书也能直接运行
- Apple 密钥仅为将来打包 `.app` 时预留

---

## 一、快速开始（无需任何密钥）

### 1.1 直接测试构建

**你不需要配置任何额外密钥即可开始测试！**

```bash
# 更新版本号（根 Cargo.toml）
# version = "0.1.14"

# 提交并推送标签
git add .
git commit -m "chore: bump version to 0.1.14"
git tag v0.1.14
git push origin v0.1.14
```

GitHub Actions 会自动：
1. ✅ 构建 Windows / macOS (x86_64 + aarch64) / Linux 的便携压缩包
2. ✅ 创建 GitHub Release 草稿

### 1.2 构建产物

| 平台 | 格式 | 状态 |
|------|------|------|
| Windows x64 | `.zip` | ✅ 完全可用 |
| macOS Intel | `.tar.gz` | ✅ 完全可用 |
| macOS Apple Silicon | `.tar.gz` | ✅ 完全可用 |
| Linux x64 | `.tar.gz` | ✅ 完全可用 |

---

## 二、配置 macOS 代码签名（预留，可选）

> 便携压缩包里的命令行二进制不受 Gatekeeper 的 `.app` 签名要求约束。
> 本节仅在未来增加 `.app` / `.dmg` 打包时才需要。

### 2.1 前提条件

- 需要 Apple Developer Program 账号（$99/年）
- 需要 macOS 电脑来生成证书

### 2.2 创建证书

1. 访问 https://developer.apple.com/account/resources/certificates/list
2. 创建 Developer ID Application 证书
3. 下载 .cer 文件，双击安装到钥匙串

### 2.3 导出为 .p12

1. 打开 macOS **钥匙串访问**
2. 找到证书，右键 → 导出
3. 选择 .p12 格式，设置密码

### 2.4 转换为 Base64

```bash
base64 -i certificate.p12 -o cert.base64
cat cert.base64  # 复制输出内容
```

### 2.5 获取签名身份

```bash
security find-identity -v -p codesigning
# 输出: Developer ID Application: Your Name (TEAM123)
```

### 2.6 获取团队 ID

访问 https://developer.apple.com/account，在 Membership 标签找到 Team ID

### 2.7 生成应用专用密码

1. 访问 https://appleid.apple.com
2. Sign-In and Security → App-Specific Passwords
3. 生成新密码（格式: xxxx-xxxx-xxxx-xxxx）

### 2.8 配置到 GitHub

在 GitHub Settings → Secrets and variables → Actions 中添加：

| 名称 | 值 | 示例 |
|------|-----|------|
| `APPLE_CERTIFICATE` | .p12 的 Base64 内容 | `MII...（很长）` |
| `APPLE_CERTIFICATE_PASSWORD` | 导出密码 | `MyPass123` |
| `APPLE_SIGNING_IDENTITY` | 签名身份 | `Developer ID Application: Your Name (TEAM123)` |
| `APPLE_ID` | Apple ID 邮箱 | `dev@example.com` |
| `APPLE_PASSWORD` | 应用专用密码 | `abcd-efgh-ijkl-mnop` |
| `APPLE_TEAM_ID` | 团队 ID | `TEAM123ABC` |

---

## 三、使用配置脚本（Windows）

我们提供了 PowerShell 脚本来帮助你配置：

```powershell
# 进入项目目录
cd .github\workflows

# 查看帮助
.\setup-secrets.ps1 -Help

# 跳过 macOS 指导
.\setup-secrets.ps1 -SkipMacOS
```

---

## 四、常见问题

### Q: 能否跳过 macOS 构建？

编辑 `.github/workflows/release.yml`，注释掉 `build-macos` job，并修改 `create-release` 的 `needs` 数组。

### Q: 构建失败怎么办？

1. 查看 Actions 标签页中的日志
2. 常见问题：
   - 依赖安装失败：重试或清除缓存
   - 编译错误：检查 Cargo.toml
   - 空间不足：使用 actions/cache

### Q: 如何验证配置是否正确？

```bash
# 推送测试标签
git tag v0.0.1-test
git push origin v0.0.1-test

# 查看 Actions 页面监控进度
# https://github.com/用户名/symbio/actions
```

---

## 五、相关链接

- [Apple 代码签名](https://developer.apple.com/support/code-signing/)
- [GitHub Actions](https://docs.github.com/actions)
