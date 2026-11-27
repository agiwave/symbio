---
kind: design
id: crates/symbio-plugin-model/README
title: Model 插件
status: active
created: 2026-07-20
impl: done
---

# Model 插件

## 职责

Model 是无状态单轮 LLM 网关：接收调用、传递消息与工具声明、流式返回结果。
它不执行工具、不维护会话历史，也不驱动义务环。

插件提供两个端口：

- `model`：带工具声明与流式事件的主模型调用。
- `model.assist`：一次性辅助问答，不带工具、不写会话历史、不写展示流。

`project_tools` 路由负责当前轮 wire 别名投影与碰撞拒绝。

## 协议与配置

内置协议为 `openai_chat`、`openai_responses`、`anthropic_messages`、`gemini_api`。
未知协议在注册或调用时明确失败，不静默回退为另一协议。

- provider 配置与 CRUD 位于 `<根>/model`；provider 明细是资源，默认 provider 在 `PLUGIN.yml`。
- `ModelProvider` 是插件私有执行契约，`BoundProvider` 是其唯一实现，不向其它插件公开为第二套模型协议。
- 会话生效 provider 按“会话选定 > 默认 provider > 首个启用 provider”解析。
- `PLUGIN.yml/default_provider_id` 只保存显式默认指针；失效时不停改磁盘指针，而是由启用项
  按 id 排序计算有效默认。因此停用、删除或重建同一 id 后，运行期与重启后的有效默认一致。
- 单次请求偏好按“有效偏好 > 本次会话生效者”解析；空值、未知 id 或停用 provider 回落到会话生效者。
- 没有有效 provider 时不登记模型端口或能力，调用点响亮失败，不猜测替代模型。

## Provider 配置写入

- 新建提交完整配置；编辑既有 Provider 提交顶层补丁，省略字段保留，显式字段替换。
  读、合并、原子写在单文件存储的同一串行边界内完成；注册表与内存镜像也在释放该锁前
  发布，删除走同一边界，避免并发写/删让磁盘与运行期视图分叉。启动加载与显式默认动作也加入
  同一存储变更锁；磁盘提交后的发布不可被调用方取消。
- 地址中的 id 是身份权威；请求中的 `id` 不改变落点。未知扩展键保留，`skip_validation`
  等控制键不落盘。
- `api_key` 的缺省、`null` 与空串都表示保留现有密钥；通用 `read` 返回空投影，执行时
  仍使用存储中的真实凭据。前端密码控件不是安全边界。
- 「测试连接」携带当前表单草稿，按补丁语义合并后只验证、不落盘；「跳过校验保存」
  经 `vdfs/action` 的 `save` 动作提交 `{ fields, skip_validation: true }`，落盘后回传新 id。
- 「设为默认 Provider」先验证目标存在且启用，再持久化 `PLUGIN.yml/default_provider_id`，
  落盘成功后才发布运行期指针并广播所有发生默认标记变化的 Provider 节点。显式指针失效时
  按启用项 id 排序计算有效默认，不把回落结果回写成第二份持久权威；重启后重算一致。

## 流式语义

- OpenAI Chat 固定发送 `stream_options.include_usage=true`。
- Anthropic 的 cache token 计入 input；Gemini 的 thoughts token 计入 output。
- 只有瞬时传输失败且未超过 3 次重采样时重试；重采样会调整 temperature，最多 1.0。
- 重试复用已收口节点 id，并撤销播种但未触碰的工具槽位。
- 非 length 原因的空流属于瞬时失败；length 截断是非瞬时失败；reasoning 内容不算空流。
- `model.assist` 的 wire 名以 Agent 结尾时，请求补一条用户侧收束指令。

## 工具投影与限制

- 内部工具身份与模型侧 wire name 分离。字符集规则只在 `src/tool_projection.rs` 中实现。
- 非 wire 字符替换为 `__`；映射不保证单射。同一 wire name 的多个内部身份在请求成形前整组拒绝。
- 会话只转交候选身份和投影结果，不在 session 中复制协议字符集规则。
- 插件不保存跨轮工具别名；别名只属于当前轮快照。
- 审批与工具执行归 tool 端口；模型输出的 `approved` 等参数不构成授权。

## 测试入口

```sh
cargo test -p symbio-plugin-model
```

测试覆盖四种协议、流式事件与用量、provider 解析、空流重试、`model.assist`、wire 投影碰撞与端口债务。
核心实现位于 `src/model_providers.rs`、`src/bound_provider.rs`、`src/assist.rs` 与 `src/tool_projection.rs`。
