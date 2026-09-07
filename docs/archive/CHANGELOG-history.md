
## 2026-04-04: LLM 配置改进

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

## 2026-04-04: 启动流程修复

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

## 2026-04-04: 文件读写工具权限修复

**问题**:

- AI 对话中调用文件读取工具时提示："路径解析后超出允许范围"

- 例如读取 `Cargo.toml` 失败，即使文件在工作区内

**原因分析**:

1. `SecurityPolicy` 在初始化时使用 `std::env::current_dir()` 作为默认工作区
2. 实际工作区路径通过 `get_workdir()` 动态获取（从 work 插件）
3. **问题**：`SecurityPolicy` 的工作区路径从未被更新，导致安全检查使用错误的路径
4. `file_read.rs` 中的路径验证逻辑没有考虑 `workspace_only` 设置，即使 `workspace_only = false`（默认值）也强制检查

**修复方案**:

1. **修改** **`file_read.rs`** **路径验证逻辑**

   - 添加 `workspace_only` 条件判断

   - 当 `workspace_only = false` 时，允许读取任意系统文件（禁止路径除外）

   - 当 `workspace_only = true` 时，只允许读取工作区内的文件

2. **动态更新** **`SecurityPolicy`** **工作区路径**

   - 在 `ToolsPlugin` 结构体中保存 `security` 引用

   - 在每次工具调用前，通过 `update_workspace_dir()` 更新工作区路径

   - 确保安全检查使用最新的、正确的工作区路径

**修改文件**:

- `src-tauri/src/plugins/agent/tools/file_read.rs`

  - 第 91 行：添加 `if self.security.workspace_only` 条件

  - 只有限制模式下才强制检查工作区路径

- `src-tauri/src/plugins/agent/tools/plugin.rs`

  - 结构体新增 `security: Arc<SecurityPolicy>` 字段

  - 构造函数保存 `security` 引用

  - `invoke()` 方法在工具调用前更新工作区路径（两处：新格式和旧格式）

**权限策略**:

- **读取**：默认允许读取系统文件（除禁止路径外），`workspace_only = true` 时只读工作区

- **写入**：始终只允许写入工作区内（`is_path_allowed_for_write` 逻辑不变）

- **禁止路径**：包含 `..` 的路径、配置的 `forbidden_paths` 始终拒绝

## 2026-04-04: AI 对话流式显示修复

**问题**:

- AI 对话在调用 Tool 后，最终显示的回复内容为空

- 流式过程中 Tool 调用显示正常，但完成后消息内容为空字符串

**原因分析**:

1. 后端 Model 插件在处理工具调用循环时，`final_content` 只在**没有工具调用**时被赋值
2. 当有工具调用时，代码执行 `break` 跳出循环的逻辑不会触发，`final_content` 保持空字符串
3. 后端最终返回 `done: true` 时，`content` 字段为空
4. 前端收到空内容后，`else if (streamingContent.value)` 条件不满足，不创建消息或创建空消息

**关键代码位置** (`plugin.rs`):

```rust
// 第 825-829 行：没有工具调用时才赋值 final_content
if tool_calls.is_empty() {
    final_content = stream_content;
    break;
}

// 第 959 行：返回最终结果时 final_content 为空
yield StreamChunk {
    data: json!({
        "content": final_content,  // 这里为空！
        "done": true
    }),
    ...
}
```

**修复方案**:

1. **添加** **`last_stream_content`** **变量**

   - 在外层循环声明 `let mut last_stream_content = String::new()`

   - 用于保存每次迭代的流式内容

2. **初始化** **`stream_content`** **时使用上次内容**

   - 在流式请求开始时，如果 `last_stream_content` 非空，则使用它初始化 `stream_content`

   - 确保多轮工具调用时内容连续

3. **工具调用完成后保存内容**

   - 在所有工具调用完成后，从消息历史中提取 assistant 消息的 content

   - 将其赋值给 `final_content`，确保最终返回的内容非空

**修改文件**:

- `src-tauri/src/plugins/agent/openai/plugin.rs`

  - 第 681 行：新增 `last_stream_content` 变量

  - 第 742-747 行：修改 `stream_content` 初始化逻辑

  - 第 924-930 行：工具调用完成后从消息历史提取 content

**技术说明**:

- 后端使用 `async_stream::stream!` 宏实现流式返回

- 工具调用循环最多执行 255 次（防止无限循环）

- 每次循环都会累积 `stream_content`，最终需要正确传递给前端

- 前端使用 `streamingContent` 响应式变量接收流式内容，完成后创建消息对象

## 2026-04-19: 工具工作区目录统一修复

**问题**:

- 部分工具（`file_edit`、`glob_search`、`content_search）使用启动时的`current\_dir()\` 作为工作区目录

- 打开新工作区后，这些工具仍然使用旧的目录，而不是新的工作区

- 不同工具之间工作区目录不同步

**原因分析**:

1. `ToolsPlugin::new()` 中创建工具时，传入 `std::env::current_dir()` 作为默认工作区
2. `FileReadTool`/`FileWriteTool`/`ShellTool` 通过 `Arc<SecurityPolicy>` 获取工作区（**动态更新**）
3. `FileEditTool`/`GlobSearchTool`/`ContentSearchTool` 持有独立的 `Arc<RwLock<PathBuf>>`（**固定不变**）
4. 虽然 `invoke()` 前会调用 `security.update_workspace_dir()`，但只更新了 `SecurityPolicy`，没有更新三个工具的独立引用

**修复方案**:

采用**共享** **`Arc<SecurityPolicy>`** 方案，让所有工具都通过 `SecurityPolicy` 获取工作区目录：

1. **修改** **`FileEditTool`**

   - 结构体：`workspace_dir: Arc<RwLock<PathBuf>>` → `security: Arc<SecurityPolicy>`

   - 构造函数：`new(workspace_dir)` → `new(security)`

   - 使用：`self.workspace_dir.read().await` → `self.security.get_workspace_dir().await`

2. **修改** **`GlobSearchTool`**

   - 同上，改为持有 `Arc<SecurityPolicy>`

3. **修改** **`ContentSearchTool`**

   - 同上，改为持有 `Arc<SecurityPolicy>`

4. **修改** **`ToolsPlugin::new()`**

   - 所有工具创建时都传入 `Arc::clone(&security)`

   - 确保所有工具共享同一个 `SecurityPolicy` 实例

**修改文件**:

- `src-tauri/src/plugins/agent/tools/file_edit.rs`

  - 结构体和构造函数修改

  - 两处 `workspace_dir` 使用改为通过 `security` 获取

- `src-tauri/src/plugins/agent/tools/glob_search.rs`

  - 结构体和构造函数修改

  - `workspace_dir` 使用改为通过 `security` 获取

- `src-tauri/src/plugins/agent/tools/content_search.rs`

  - 结构体和构造函数修改

  - 两处 `workspace_dir` 使用改为通过 `security` 获取

- `src-tauri/src/plugins/agent/tools/plugin.rs`

  - 工具创建时统一使用 `Arc::clone(&security)`

**架构优势**:

- ✅ **单一数据源**：所有工具的工作区目录来自同一个 `SecurityPolicy`

- ✅ **自动同步**：调用 `security.update_workspace_dir()` 后，所有工具立即生效

- ✅ **代码简化**：减少重复的 `Arc<RwLock<PathBuf>>` 管理

- ✅ **易于扩展**：新增工具只需传入 `Arc<SecurityPolicy>` 即可

**当前目录切换说明**:

- 现在项目**已使用** `std::env::set_current_dir()` 在打开工作区后切换进程当前目录

- 切换时机：

  1. **应用启动时**：根据配置文件中的 `workdir` 切换当前目录
  2. **用户选择新工作区时**：调用 `set_workspace` 后立即切换当前目录

- 所有工具通过显式路径拼接（`workspace_dir.join(path)`）或命令参数（`cmd.current_dir()`）使用工作区

- 当前目录切换是**额外的便利功能**，工具仍然通过 `SecurityPolicy` 获取工作区目录，确保安全

## 2026-04-04: Windows 路径规范化修复

**问题**:

- 在 Windows 上，`tokio::fs::canonicalize()` 返回带有 `\\?\` 前缀的路径（如 `\\?\C:\Bing\agiwave\symbio\docs`）

- 而 `workspace_dir` 没有这个前缀（如 `C:\Bing\agiwave\symbio`）

- 导致 `starts_with` 比较失败，即使路径实际上在工作区内

**原因分析**:

- Windows 的 `canonicalize` 返回 UNC 格式路径（`\\?\` 前缀）

- 直接字符串或路径比较会失败，因为前缀不一致

**修复方案**:

在 `policy.rs` 中添加路径规范化辅助函数：

```rust
/// 规范化路径用于比较
/// 在 Windows 上，canonicalize 返回带有 `\\?\` 前缀的路径，需要统一处理
pub fn normalize_path_for_comparison(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    // 移除 Windows UNC 路径前缀（如 \\?\）
    if path_str.starts_with("\\\\?\\") {
        PathBuf::from(&path_str[4..])
    } else {
        path.to_path_buf()
    }
}

/// 检查路径是否以另一个路径为前缀（规范化后比较）
pub fn path_starts_with_normalized(base: &Path, prefix: &Path) -> bool {
    let normalized_base = normalize_path_for_comparison(base);
    let normalized_prefix = normalize_path_for_comparison(prefix);
    normalized_base.starts_with(&normalized_prefix)
}
```

**修改文件**:

- `src-tauri/src/plugins/agent/tools/policy.rs` - 添加路径规范化函数，修改 `is_path_allowed` 和 `is_path_allowed_for_write`

- `src-tauri/src/plugins/agent/tools/file_edit.rs` - 使用 `path_starts_with_normalized`

- `src-tauri/src/plugins/agent/tools/file_read.rs` - 使用 `path_starts_with_normalized`

- `src-tauri/src/plugins/agent/tools/glob_search.rs` - 使用 `path_starts_with_normalized` 和 `normalize_path_for_comparison`

- `src-tauri/src/plugins/agent/tools/content_search.rs` - 使用 `path_starts_with_normalized`

**影响范围**:

- 所有使用 `canonicalize` 后进行路径比较的工具都得到修复

- 包括：`file_edit`、`file_read`、`glob_search`、`content_search`

- `SecurityPolicy` 中的路径验证也得到修复

