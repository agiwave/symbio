# Prompt 形状基线（出厂 `full` 路径）

> ⚠️ **生成物，不手改。** 唯一写者是 `node e2e/cases/t46-prompt-shape-baseline.mjs`
> 带 `PROMPT_SHAPE_UPDATE=1`；判据由那条用例在门禁里每次复算（改了装配就红），
> 口径与「什么进基线、什么只作观测」见本脚本文件头，
> 提取实现在 `scripts/audits/prompt-shape-audit.mjs`。

场景 `full-主线装配-工具轮-对话面关闭` · 重生成于 2026-11-27 · 请求数 3

## 判据列（逐字段比，PS-001）

| 请求 | 消息数 | 消息序列（role + 结构载体） | system 小节标签序列 | 工具数 |
|---|---|---|---|---|
| req0 | 2 | system → user | 【工作区记忆】【会话记忆】  | 19 |
| req1 | 4 | system → user → assistant[calls=1] → tool[id] | 【工作区记忆】【会话记忆】    | 19 |
| req2 | 6 | system → user → assistant[calls=1] → tool[id] → assistant → user | 【工作区记忆】【会话记忆】【长期记忆】      | 19 |

## 规模观测（**不进判据**：同一列里既有装配代码的字，也有负载的字）

正文规模同时被**用例文本**与**召回内容**决定，钉进判据会让「改用例措辞」变红，
而那只与装配无关。下表是重生成当时的一次快照，两次快照之差 = 那次结构改动的代价。

| 请求 | 消息数 | 消息序列 | system 小节标签 | 工具数 | 正文字节 | 工具 schema 字节 | 请求体总字节 |
|---|---|---|---|---|---|---|---|
| req0 | 2 | system → user | 【工作区记忆】【会话记忆】  | 19 | 1514 | 13841 | 15598 |
| req1 | 4 | system → user → assistant[calls=1] → tool[id] | 【工作区记忆】【会话记忆】    | 19 | 1570 | 13841 | 15890 |
| req2 | 6 | system → user → assistant[calls=1] → tool[id] → assistant → user | 【工作区记忆】【会话记忆】【长期记忆】      | 19 | 2004 | 13841 | 16391 |

## 基线（判据本体）

```json:prompt-shape
{
  "schema": 2,
  "scenario": "full-主线装配-工具轮-对话面关闭",
  "requests": [
    {
      "index": 0,
      "envelope": {
        "model": "mock-model",
        "stream": true,
        "temperature": 0.1,
        "tool_choice": "auto",
        "max_tokens": 1024
      },
      "message_shape": [
        {
          "role": "system",
          "tool_calls": 0,
          "tool_call_id": false,
          "content": "text"
        },
        {
          "role": "user",
          "tool_calls": 0,
          "tool_call_id": false,
          "content": "text"
        }
      ],
      "system_sections": [
        [
          {
            "tag": "工作区记忆",
            "chars": 151
          },
          {
            "tag": "会话记忆",
            "chars": 567
          }
        ],
        []
      ],
      "tools": {
        "count": 19,
        "names": [
          "<shell>",
          "content_search",
          "todo_write",
          "codebase_search",
          "ask_user",
          "mcp__mockserv__echo",
          "mcp__mockserv__add",
          "agent_run",
          "vdfs_list",
          "vdfs_stat",
          "vdfs_read",
          "vdfs_edit",
          "vdfs_search",
          "vdfs_write",
          "vdfs_delete",
          "vdfs_mkdir",
          "web_fetch",
          "web_search",
          "http_request"
        ],
        "schema_bytes": 13841
      }
    },
    {
      "index": 1,
      "envelope": {
        "model": "mock-model",
        "stream": true,
        "temperature": 0.1,
        "tool_choice": "auto",
        "max_tokens": 1024
      },
      "message_shape": [
        {
          "role": "system",
          "tool_calls": 0,
          "tool_call_id": false,
          "content": "text"
        },
        {
          "role": "user",
          "tool_calls": 0,
          "tool_call_id": false,
          "content": "text"
        },
        {
          "role": "assistant",
          "tool_calls": 1,
          "tool_call_id": false,
          "content": "text"
        },
        {
          "role": "tool",
          "tool_calls": 0,
          "tool_call_id": true,
          "content": "text"
        }
      ],
      "system_sections": [
        [
          {
            "tag": "工作区记忆",
            "chars": 151
          },
          {
            "tag": "会话记忆",
            "chars": 567
          }
        ],
        [],
        [],
        []
      ],
      "tools": {
        "count": 19,
        "names": [
          "<shell>",
          "content_search",
          "todo_write",
          "codebase_search",
          "ask_user",
          "mcp__mockserv__echo",
          "mcp__mockserv__add",
          "agent_run",
          "vdfs_list",
          "vdfs_stat",
          "vdfs_read",
          "vdfs_edit",
          "vdfs_search",
          "vdfs_write",
          "vdfs_delete",
          "vdfs_mkdir",
          "web_fetch",
          "web_search",
          "http_request"
        ],
        "schema_bytes": 13841
      }
    },
    {
      "index": 2,
      "envelope": {
        "model": "mock-model",
        "stream": true,
        "temperature": 0.1,
        "tool_choice": "auto",
        "max_tokens": 1024
      },
      "message_shape": [
        {
          "role": "system",
          "tool_calls": 0,
          "tool_call_id": false,
          "content": "text"
        },
        {
          "role": "user",
          "tool_calls": 0,
          "tool_call_id": false,
          "content": "text"
        },
        {
          "role": "assistant",
          "tool_calls": 1,
          "tool_call_id": false,
          "content": "text"
        },
        {
          "role": "tool",
          "tool_calls": 0,
          "tool_call_id": true,
          "content": "text"
        },
        {
          "role": "assistant",
          "tool_calls": 0,
          "tool_call_id": false,
          "content": "text"
        },
        {
          "role": "user",
          "tool_calls": 0,
          "tool_call_id": false,
          "content": "text"
        }
      ],
      "system_sections": [
        [
          {
            "tag": "工作区记忆",
            "chars": 151
          },
          {
            "tag": "会话记忆",
            "chars": 146
          },
          {
            "tag": "长期记忆",
            "chars": 560
          }
        ],
        [],
        [],
        [],
        [],
        []
      ],
      "tools": {
        "count": 19,
        "names": [
          "<shell>",
          "content_search",
          "todo_write",
          "codebase_search",
          "ask_user",
          "mcp__mockserv__echo",
          "mcp__mockserv__add",
          "agent_run",
          "vdfs_list",
          "vdfs_stat",
          "vdfs_read",
          "vdfs_edit",
          "vdfs_search",
          "vdfs_write",
          "vdfs_delete",
          "vdfs_mkdir",
          "web_fetch",
          "web_search",
          "http_request"
        ],
        "schema_bytes": 13841
      }
    }
  ]
}
```
