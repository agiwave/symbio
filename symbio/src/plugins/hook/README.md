# Hook 插件

钩子插件：在生命周期关键点（如会话压缩前 PreCompact）注册并触发钩子。

## 路由

| Path | 说明 |
|------|------|
| `hook/register` | 注册钩子 |
| `hook/list` | 列出已注册钩子 |
| `hook/trigger` | 触发钩子（也可由系统内部在生命周期点调用） |

## 机制

- 钩子是"在特定事件点执行附加逻辑"的扩展点；session 在主动压缩前会触发 PreCompact 钩子（见 `../session/README.md` 策略⑥）。
- 钩子执行结果不影响主流程终态，仅作信息性附加。

## 关联

- 压缩生命周期：`../session/README.md`
