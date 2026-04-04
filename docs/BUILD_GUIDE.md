# 编译与开发环境指南

## 系统要求

- **操作系统**: Windows 10/11
- **Node.js**: >= 18.0.0
- **Rust**: 1.88.0（推荐稳定版）
- **CPU**: 所有 x86_64 处理器（特殊配置已处理第 15 代 Intel CPU 兼容性问题）

## 快速开始

```bash
# 安装依赖
npm install

# 运行开发模式
npm run tauri dev

# 构建生产版本
npm run build
npm run tauri build
```

## 已知问题与解决方案

### 1. Rust 编译器崩溃 (STATUS_ACCESS_VIOLATION / STATUS_ILLEGAL_INSTRUCTION)

**症状**:
```
error: could not compile `webview2-com-sys` (lib)
Caused by:
  process didn't exit successfully: `rustc.exe ...` (exit code: 0xc0000005, STATUS_ACCESS_VIOLATION)
```
或
```
error: could not compile `syn` (lib)
Caused by:
  process didn't exit successfully: `rustc.exe ...` (exit code: 0xc000001d, STATUS_ILLEGAL_INSTRUCTION)
```

**原因**: 
- Intel 第 15 代处理器（Arrow Lake，如 Core Ultra 7 265K）与 Rust 编译器的指令集兼容性问题
- 增量编译可能导致缓存损坏

**解决方案**:
已在 `src-tauri/.cargo/config.toml` 中配置：
```toml
[build]
rustflags = ["-C", "target-cpu=x86-64-v2", "-C", "prefer-dynamic"]
```

并在 `src-tauri/Cargo.toml` 中禁用增量编译：
```toml
[profile.dev]
incremental = false
opt-level = 0
```

**如果问题再次出现**:
```bash
# 彻底清理编译缓存
cd src-tauri
rmdir /s /q target
cd %USERPROFILE%\.cargo
rmdir /s /q registry

# 重新编译
cd C:\bing\agiwave\symbio
npm run tauri dev
```

### 2. Vite 版本不兼容

**症状**:
```
error during build:
Error: Failed to load `transformWithEsbuild`. It is deprecated...
Cannot find package 'esbuild'
```

**原因**: 
- Vite 8.x 与 `@vitejs/plugin-vue@5` 不兼容
- 需要 Vite 6.x 或 7.x

**解决方案**:
已在 `package.json` 中锁定版本：
```json
{
  "devDependencies": {
    "vite": "^6.0.0"
  }
}
```

**不要**将 Vite 升级到 8.x，除非同时升级 `@vitejs/plugin-vue` 到兼容版本。

### 3. Tauri 前端路径错误

**症状**:
```
error: proc macro panicked
  --> src\main.rs:73:14
   |
73 |         .run(tauri::generate_context!())
   |              ^^^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = help: message: The `frontendDist` configuration is set to `"../dist"` but this path doesn't exist
```

**原因**: 
- Tauri 2.x 的 `generate_context!()` 宏在编译时需要 `frontendDist` 路径存在
- 在 `npm run tauri dev` 模式下，虽然实际使用 `devUrl`，但路径检查仍然执行

**解决方案**:
首次运行或 `dist` 目录被删除时，先构建前端：
```bash
npm run build
npm run tauri dev
```

### 4. Cargo 依赖版本冲突

**症状**:
```
npm error ERESOLVE could not resolve
npm error While resolving: @vitejs/plugin-vue@5.2.4
npm error Found: vite@8.0.3
```

**解决方案**:
使用 `--legacy-peer-deps` 或保持依赖版本兼容：
```bash
npm install --legacy-peer-deps
```

## 依赖版本锁定

以下为经过测试的兼容版本组合（记录于 2026-04-04）：

| 依赖 | 版本 | 说明 |
|------|------|------|
| rustc | 1.88.0 | 稳定版，避免使用 nightly |
| vite | ^6.0.0 | 不要升级到 8.x |
| @vitejs/plugin-vue | ^5.0.0 | 与 Vite 6 兼容 |
| @tauri-apps/cli | ^2.0.0 | Tauri 2.x |
| @tauri-apps/api | ^2.0.0 | 与 CLI 版本一致 |

## 开发工作流

### 日常开发

```bash
# 1. 确保前端构建过（首次或 dist 被删除时）
npm run build

# 2. 运行开发模式（自动监听前后端变化）
npm run tauri dev
```

### 清理与重建

```bash
# 清理 Rust编译缓存
cd src-tauri && cargo clean

# 清理 Node 模块
cd .. && rm -rf node_modules
npm install

# 完全重建
npm run build
npm run tauri dev
```

### 调试技巧

```bash
# 查看编译详细输出
cd src-tauri && cargo build -vv

# 检查 Rust 工具链版本
rustc --version
# 应该输出: rustc 1.88.0 (...)

# 检查 Tauri 状态
npm run tauri info
```

## 故障排除检查清单

- [ ] Rust 工具链是 1.88.0 稳定版（`rustc --version`）
- [ ] `src-tauri/.cargo/config.toml` 存在且包含正确的 `rustflags`
- [ ] `src-tauri/Cargo.toml` 中 `[profile.dev]` 设置 `incremental = false`
- [ ] Vite 版本是 6.x（`npm list vite`）
- [ ] `dist` 目录存在（运行过 `npm run build`）
- [ ] 没有防病毒软件拦截 Rust 编译器

## 功能更新记录

### 2026-04-04: LLM 配置改进

**新增功能**:
1. **LM Studio 支持**
   - 在 LLM 提供商下拉列表新增 "LM Studio" 选项
   - 默认 API 地址: `http://localhost:1234/v1`
   - 模型列表为空，支持用户手动输入模型名称

2. **模型名称可输入**
   - 将模型选择框从 `<select>` 改为 `<input list="models-list">`
   - 支持从预设列表选择（有建议列表）
   - 支持手动输入任意模型名称（无限制）
   - 适用于 LM Studio 等动态加载模型的场景

**修改文件**:
- `src/components/SettingsPage.vue`
  - 新增 LM Studio 提供商预设
  - 模型输入框改用 datalist 实现可选择可输入
  - 优化样式和交互逻辑

**技术说明**:
- 使用 HTML5 `<datalist>` 元素实现自动完成输入框
- LM Studio 的 `models` 设为空数组，因为模型是动态加载的
- 后端 token.rs 的 `get_model_config()` 会自动为未知模型使用默认配置

### 2026-04-04: 启动流程修复

**问题**:
- App 启动时直接进入导航页面，跳过了工作区选择页面
- 原因：`loadWorkspaceState()` 在检测到有效工作区路径时直接设置 `workspaceReady = true`

**修复**:
1. **始终显示欢迎页面**
   - 修改 `loadWorkspaceState()` 逻辑，无论是否有有效工作区路径，都设置 `workspaceReady = false`
   - 每次启动都从欢迎页面开始

2. **新增"继续上次工作区"功能**
   - 如果有上次使用的工作区路径，显示蓝色的"继续"按钮
   - 按钮显示路径信息，方便用户确认
   - 用户可以选择：
     - 点击"继续上次工作区"快速进入
     - 点击"浏览目录"选择新的工作区
     - 从最近使用列表中选择

**修改文件**:
- `src/views/HomeView.vue`
  - 修改 `loadWorkspaceState()` 函数逻辑
  - 新增 `continueLastWorkspace()` 函数
  - 模板中添加条件渲染的"继续"按钮
  - 添加 `.continue-btn` 样式

**技术说明**:
- 欢迎页面通过 `v-if="!workspaceReady"` 控制显示
- 工作区路径保存在后端配置文件中
- 最近使用列表最多保存 5 条记录

## 参考资源

- [Tauri 2.0 文档](https://v2.tauri.app/)
- [Rust 工具链管理](https://rustup.rs/)
- [Vite 配置参考](https://vitejs.dev/config/)
