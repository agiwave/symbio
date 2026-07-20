# GitHub Actions 密钥配置指南

本文档详细说明如何生成和配置 GitHub Actions 所需的所有密钥，以实现自动打包各平台应用。

---

## 密钥清单

| 密钥名称 | 用途 | 必需 | 平台 |
|---------|------|------|------|
| `GITHUB_TOKEN` | GitHub 自动提供，用于创建 Release | ✅ 是 | 全部 |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri 自动更新签名 | ❌ 可选 | 全部 |
| `APPLE_CERTIFICATE` | macOS 代码签名证书 | ❌ 可选 | macOS |
| `APPLE_CERTIFICATE_PASSWORD` | 证书密码 | ❌ 可选 | macOS |
| `APPLE_SIGNING_IDENTITY` | Apple 签名身份 | ❌ 可选 | macOS |
| `APPLE_ID` | Apple ID（用于公证） | ❌ 可选 | macOS |
| `APPLE_PASSWORD` | Apple 应用专用密码 | ❌ 可选 | macOS |
| `APPLE_TEAM_ID` | Apple 开发者团队 ID | ❌ 可选 | macOS |

**说明**:
- ✅ **GITHUB_TOKEN** 由 GitHub 自动提供，无需手动配置
- 不配置其他密钥也能正常构建，但 macOS 会有未签名警告
- macOS 未签名时，用户需要手动绕过 Gatekeeper（见文档底部）

---

## 一、快速开始（无需任何密钥）

### 1.1 直接测试构建

**你不需要配置任何额外密钥即可开始测试！**

```bash
# 更新版本号（三个文件）
# - package.json: "version": "0.2.0"
# - src-tauri/tauri.conf.json: "version": "0.2.0"
# - src-tauri/Cargo.toml: version = "0.2.0"

# 提交并推送标签
git add .
git commit -m "chore: bump version to 0.2.0"
git tag v0.2.0
git push origin v0.2.0
```

GitHub Actions 会自动：
1. ✅ 构建 Windows (x64) NSIS/MSI 安装包
2. ✅ 构建 macOS DMG（未签名，有警告）
3. ✅ 构建 Linux AppImage/DEB
4. ✅ 创建 GitHub Release 草稿

### 1.2 构建产物

| 平台 | 格式 | 状态 |
|------|------|------|
| Windows x64 | `.exe` (NSIS), `.msi` | ✅ 完全可用 |
| macOS Intel | `.dmg` | ⚠️ 未签名（可运行） |
| macOS Apple Silicon | `.dmg` | ⚠️ 未签名（可运行） |
| Linux x64 | `.AppImage`, `.deb` | ✅ 完全可用 |

---

## 二、配置 Tauri 更新签名密钥（可选）

如果需要 Tauri 自动更新功能：

### 2.1 生成密钥

```bash
# 安装 Tauri CLI
npm install -g @tauri-apps/cli

# 生成密钥
cd C:\bing\agiwave\symbio
tauri signer generate
```

按提示输入密码（可为空）。

### 2.2 获取私钥

```powershell
# Windows PowerShell
type $env:USERPROFILE\.tauri\.key
```

### 2.3 配置到 GitHub

1. 打开: `https://github.com/用户名/symbio/settings/secrets/actions`
2. 点击 **New repository secret**
3. Name: `TAURI_SIGNING_PRIVATE_KEY`
4. Secret: 粘贴私钥内容

---

## 三、配置 macOS 代码签名（可选但推荐）

### 3.1 前提条件

- 需要 Apple Developer Program 账号（$99/年）
- 需要 macOS 电脑来生成证书

### 3.2 创建证书

#### 方法 1: 使用 Xcode（最简单）

1. 打开 Xcode → Settings → Accounts
2. 添加 Apple ID
3. Manage Certificates → + → Developer ID Application
4. 下载并安装证书

#### 方法 2: Apple Developer 网站

1. 访问 https://developer.apple.com/account/resources/certificates/list
2. 创建 Developer ID Application 证书
3. 下载 .cer 文件，双击安装到钥匙串

### 3.3 导出为 .p12

1. 打开 macOS **钥匙串访问**
2. 找到证书，右键 → 导出
3. 选择 .p12 格式，设置密码

### 3.4 转换为 Base64

```bash
base64 -i certificate.p12 -o cert.base64
cat cert.base64  # 复制输出内容
```

### 3.5 获取签名身份

```bash
security find-identity -v -p codesigning
# 输出: Developer ID Application: Your Name (TEAM123)
```

### 3.6 获取团队 ID

访问 https://developer.apple.com/account，在 Membership 标签找到 Team ID

### 3.7 生成应用专用密码

1. 访问 https://appleid.apple.com
2. Sign-In and Security → App-Specific Passwords
3. 生成新密码（格式: xxxx-xxxx-xxxx-xxxx）

### 3.8 配置到 GitHub

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

## 四、macOS 未签名版本的用户指南

如果不配置 macOS 签名证书，用户下载后需要手动信任：

### 方法 1: 右键打开

1. 右键点击应用
2. 按住 Option 键
3. 选择"打开"
4. 点击"仍要打开"

### 方法 2: 终端命令

```bash
xattr -rd com.apple.quarantine /Applications/Symbio.app
```

### 方法 3: 系统设置

系统偏好设置 → 隐私与安全性 → 仍要打开

---

## 五、使用配置脚本（Windows）

我们提供了 PowerShell 脚本来帮助你配置：

```powershell
# 进入项目目录
cd C:\bing\agiwave\symbio\.github\workflows

# 查看帮助
.\setup-secrets.ps1 -Help

# 生成 Tauri 签名密钥
.\setup-secrets.ps1 -GenerateTauriKey

# 跳过 macOS 指导
.\setup-secrets.ps1 -SkipMacOS
```

---

## 六、常见问题

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

## 七、相关链接

- [Tauri CI/CD 文档](https://tauri.app/distribute/pipelines/)
- [Tauri 签名文档](https://tauri.app/distribute/sign/)
- [Apple 代码签名](https://developer.apple.com/support/code-signing/)
- [GitHub Actions](https://docs.github.com/actions)
