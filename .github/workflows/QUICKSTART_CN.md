# 🚀 自动构建指南

## 快速开始（5 分钟）

### 1️⃣ 更新版本号

修改三个文件中的版本号：

```json
// package.json
{
  "version": "0.2.0"
}

// src-tauri/tauri.conf.json
{
  "version": "0.2.0"
}

// src-tauri/Cargo.toml
version = "0.2.0"
```

### 2️⃣ 提交并推送标签

```bash
git add .
git commit -m "chore: bump version to 0.2.0"
git tag v0.2.0
git push origin v0.2.0
```

### 3️⃣ 等待自动构建

推送标签后，GitHub Actions 会自动：

- ✅ 在 Windows/macOS/Linux 上并行构建
- ✅ 生成各平台安装包
- ✅ 创建 GitHub Release 草稿

约 20-40 分钟后，在 **GitHub → Releases** 查看构建产物！

---

## 📦 构建产物

| 平台 | 格式 | 说明 |
|------|------|------|
| **Windows x64** | `.exe` (NSIS), `.msi` | 安装包 |
| **macOS Intel** | `.dmg` | 未签名，需手动信任 |
| **macOS Apple Silicon** | `.dmg` | M1/M2，未签名 |
| **Linux x64** | `.AppImage`, `.deb` | 便携包/DEB包 |

---

## ⚙️ 可选配置

### Tauri 自动更新签名

```bash
npm install -g @tauri-apps/cli
tauri signer generate

# 将生成的私钥添加到 GitHub Secrets:
# TAURI_SIGNING_PRIVATE_KEY
```

### macOS 代码签名（消除警告）

需要 Apple Developer 账号，配置 6 个密钥：

| 密钥 | 说明 |
|------|------|
| `APPLE_CERTIFICATE` | .p12 证书的 Base64 |
| `APPLE_CERTIFICATE_PASSWORD` | 证书密码 |
| `APPLE_SIGNING_IDENTITY` | 签名身份 |
| `APPLE_ID` | Apple ID 邮箱 |
| `APPLE_PASSWORD` | 应用专用密码 |
| `APPLE_TEAM_ID` | 团队 ID |

详细配置指南: [SECRETS_SETUP.md](.github/workflows/SECRETS_SETUP.md)

---

## 🔧 手动触发构建

1. 进入 GitHub → Actions
2. 选择 **Release** 工作流
3. 点击 **Run workflow**
4. 勾选 "创建 GitHub Release"
5. 点击运行

---

## 📖 完整文档

- [密钥配置指南](.github/workflows/SECRETS_SETUP.md)
- [工作流配置](.github/workflows/release.yml)
- [配置脚本](.github/workflows/setup-secrets.ps1)

---

## ❓ 常见问题

**Q: 需要配置密钥才能构建吗？**

A: 不需要！直接推送标签即可构建，只是 macOS 版本会有未签名警告。

**Q: macOS 未签名怎么用？**

A: 右键点击应用 → 按住 Option → 打开 → 仍要打开

**Q: 构建失败怎么办？**

A: 查看 Actions 页面日志，常见原因：依赖安装失败、编译错误

**Q: 如何跳过 macOS 构建？**

A: 编辑 `.github/workflows/release.yml`，注释掉 `build-macos` job
