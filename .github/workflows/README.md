# CI/CD 配置说明

## GitHub Actions 工作流

### release.yml - 自动构建和发布

#### 触发条件

1. **推送标签**: 当推送以 `v` 开头的标签时自动触发
   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

2. **手动触发**: 在 GitHub Actions 页面手动触发工作流

#### 构建目标

| 平台 | 架构 | 输出格式 |
|------|------|----------|
| Windows | x64 | NSIS (.exe), MSI (.msi) |
| Windows | ARM64 | NSIS (.exe), MSI (.msi) |
| macOS | Intel (x86_64) | DMG (.dmg) |
| macOS | Apple Silicon (aarch64) | DMG (.dmg) |
| Linux | x64 | AppImage, DEB |
| Linux | ARM64 | AppImage, DEB |

#### 构建产物

- 所有构建产物会作为 Artifacts 上传
- 如果通过标签触发，会自动创建 GitHub Release（草稿状态）
- Release 包含所有平台的安装包和更新日志

#### 手动触发步骤

1. 进入 GitHub 仓库页面
2. 点击 **Actions** 标签
3. 选择 **Release** 工作流
4. 点击 **Run workflow** 按钮
5. 勾选 "创建 GitHub Release" 选项
6. 点击 **Run workflow**

#### 发布新版本

```bash
# 1. 更新版本号（三处需要同步更新）
# - package.json: "version": "0.2.0"
# - src-tauri/tauri.conf.json: "version": "0.2.0"
# - src-tauri/Cargo.toml: version = "0.2.0"

# 2. 提交更改
git add .
git commit -m "chore: bump version to 0.2.0"

# 3. 创建并推送标签
git tag v0.2.0
git push origin v0.2.0

# 4. GitHub Actions 会自动开始构建
# 5. 构建完成后，在 Releases 页面检查并正式发布
```

## 密钥配置（可选）

以下密钥在 GitHub Settings → Secrets and variables → Actions 中配置：

| 密钥名称 | 说明 | 必需 |
|---------|------|------|
| `GITHUB_TOKEN` | GitHub 自动提供，用于创建 Release | ✅ |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri 更新签名私钥 | ❌ |
| `APPLE_CERTIFICATE` | Apple 开发者证书（Base64） | ❌ |
| `APPLE_CERTIFICATE_PASSWORD` | 证书密码 | ❌ |
| `APPLE_SIGNING_IDENTITY` | Apple 签名身份 | ❌ |
| `APPLE_ID` | Apple ID（用于公证） | ❌ |
| `APPLE_PASSWORD` | Apple ID 密码（应用专用密码） | ❌ |
| `APPLE_TEAM_ID` | Apple 团队 ID | ❌ |

**注意**: 
- 不配置密钥也能正常构建，但 macOS 版本会有未签名警告
- `GITHUB_TOKEN` 由 GitHub 自动提供，无需手动配置
- 首次构建可能需要 20-40 分钟（取决于缓存）

## 自定义构建配置

编辑 `.github/workflows/release.yml` 文件可以：

- 添加/删除构建目标
- 修改打包格式（如添加 zip）
- 调整发布说明模板
- 添加测试步骤

## 故障排查

### 构建失败

1. 检查 Actions 标签页中的日志
2. 常见问题：
   - 依赖安装失败：尝试清除缓存后重试
   - Rust 编译错误：检查 Cargo.toml 配置
   - 磁盘空间不足：使用 `actions/cache` 优化

### macOS 构建警告

如果不配置 Apple 证书，macOS 版本会显示"无法验证开发者"。用户需要：
```bash
# 在终端运行以下命令绕过 Gatekeeper
xattr -rd com.apple.quarantine /Applications/Symbio.app
```

### 版本不一致

工作流会检查 tag 版本与 Cargo.toml 版本是否一致。如果收到警告：
```bash
# 检查版本号
grep '^version = ' src-tauri/Cargo.toml
cat package.json | grep version
cat src-tauri/tauri.conf.json | grep version
```
