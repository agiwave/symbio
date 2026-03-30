# 任务索引

> 本文档提供所有任务的快速索引

## 项目概览

| 指标 | 数值 |
|------|------|
| 总任务数 | 24 |
| 已完成 | 7 |
| 待处理 | 17 |
| 预估工时 | 132h |

---

## 按模块查看

### 基础架构 (Infrastructure)

| 任务ID | 标题 | 优先级 | 预估工时 | 状态 |
|--------|------|--------|----------|------|
| T001 | 项目基础架构搭建 | P0 | - | ✅ completed |
| T026 | 插件架构重构 | P0 | - | ✅ completed |

### 工作区模块 (Workspace)

| 任务ID | 标题 | 优先级 | 预估工时 | 状态 | 依赖 |
|--------|------|--------|----------|------|------|
| T002 | Markdown 编辑器集成 | P0 | - | ✅ completed | - |
| T003 | 文档树管理 | P0 | - | ✅ completed | - |
| T004 | Docker 执行环境搭建 | P0 | - | ✅ completed | - |
| T012 | 三栏布局 UI 实现 | P0 | - | ✅ completed | - |
| T013 | 导航与工作区切换 | P0 | - | ✅ completed | - |
| W001 | 文档持久化存储 | P0 | 8h | pending | T026 |
| W002 | 文档导入导出 | P1 | 6h | pending | W001 |
| W003 | 代码块执行引擎 | P0 | 16h | pending | T004 |
| W004 | 执行结果展示 | P0 | 12h | pending | W003 |
| W005 | RNA-seq 分析模板 | P1 | 16h | pending | W003 |

**工作区模块小计**: 58h

### 智能体模块 (Agent)

| 任务ID | 标题 | 优先级 | 预估工时 | 状态 | 依赖 |
|--------|------|--------|----------|------|------|
| A001 | AI 对话接口集成 | P0 | 12h | pending | T026 |
| A002 | AI 对话 UI 完善 | P0 | 8h | pending | A001 |
| A003 | AI 悬浮输入框 | P1 | 8h | pending | A001 |
| A004 | AI 错误诊断 | P1 | 12h | pending | A001, W003 |
| A005 | AI 代码解释 | P1 | 8h | pending | A001 |

**智能体模块小计**: 48h

### 设置模块 (Setting)

| 任务ID | 标题 | 优先级 | 预估工时 | 状态 | 依赖 |
|--------|------|--------|----------|------|------|
| S001 | 设置持久化 | P1 | 6h | pending | T026 |
| S002 | 外观设置 | P1 | 8h | pending | S001 |
| S003 | AI 提供商配置 | P0 | 6h | pending | S001, A001 |
| S004 | Docker 环境配置 | P1 | 6h | pending | S001 |
| S005 | 数据管理 | P1 | 6h | pending | S001 |

**设置模块小计**: 32h

---

## 按优先级查看

### P0 (必须完成)

| 任务ID | 标题 | 模块 | 状态 |
|--------|------|------|------|
| T001 | 项目基础架构搭建 | infrastructure | ✅ completed |
| T002 | Markdown 编辑器集成 | workspace | ✅ completed |
| T003 | 文档树管理 | workspace | ✅ completed |
| T004 | Docker 执行环境搭建 | workspace | ✅ completed |
| T012 | 三栏布局 UI 实现 | workspace | ✅ completed |
| T013 | 导航与工作区切换 | workspace | ✅ completed |
| T026 | 插件架构重构 | infrastructure | ✅ completed |
| W001 | 文档持久化存储 | workspace | pending |
| W003 | 代码块执行引擎 | workspace | pending |
| W004 | 执行结果展示 | workspace | pending |
| A001 | AI 对话接口集成 | agent | pending |
| A002 | AI 对话 UI 完善 | agent | pending |
| S003 | AI 提供商配置 | setting | pending |

**P0 已完成**: 7 / 13

### P1 (重要)

| 任务ID | 标题 | 模块 | 状态 |
|--------|------|------|------|
| W002 | 文档导入导出 | workspace | pending |
| W005 | RNA-seq 分析模板 | workspace | pending |
| A003 | AI 悬浮输入框 | agent | pending |
| A004 | AI 错误诊断 | agent | pending |
| A005 | AI 代码解释 | agent | pending |
| S001 | 设置持久化 | setting | pending |
| S002 | 外观设置 | setting | pending |
| S004 | Docker 环境配置 | setting | pending |
| S005 | 数据管理 | setting | pending |

**P1 合计**: 9 个待处理

---

## 依赖关系图

```
已完成的任务
═══════════════════════════════════════════
T001 (基础架构) ─┬─ T002 (编辑器) ✅
                 ├─ T003 (文档树) ✅
                 ├─ T004 (Docker) ✅
                 ├─ T012 (三栏布局) ✅
                 ├─ T013 (导航) ✅
                 └─ T026 (插件架构) ✅

待处理的任务
═══════════════════════════════════════════
T026 (插件架构) ─┬─ W001 (文档持久化)
                │   └─ W002 (导入导出)
                │
                ├─ A001 (AI接口)
                │   ├─ A002 (AI UI)
                │   ├─ A003 (悬浮输入)
                │   ├─ A004 (错误诊断) ← W003
                │   └─ A005 (代码解释)
                │
                └─ S001 (设置持久化)
                    ├─ S002 (外观设置)
                    ├─ S003 (AI配置) ← A001
                    ├─ S004 (Docker配置)
                    └─ S005 (数据管理)

T004 (Docker) ─── W003 (执行引擎)
                  ├─ W004 (结果展示)
                  └─ W005 (RNA-seq模板)
```

---

## 开发路线

### Sprint 1: 工作区核心 (当前)

**目标**: 完善工作区基础能力

| 任务 | 预估 | 说明 |
|------|------|------|
| W001 | 8h | 文档持久化存储 |
| W003 | 16h | 代码块执行引擎 |
| W004 | 12h | 执行结果展示 |

**合计**: 36h

### Sprint 2: 智能体核心

**目标**: AI 对话能力可用

| 任务 | 预估 | 说明 |
|------|------|------|
| A001 | 12h | AI 对话接口集成 |
| A002 | 8h | AI 对话 UI 完善 |
| S003 | 6h | AI 提供商配置 |

**合计**: 26h

### Sprint 3: 设置完善

**目标**: 设置功能基本可用

| 任务 | 预估 | 说明 |
|------|------|------|
| S001 | 6h | 设置持久化 |
| S002 | 8h | 外观设置 |
| S004 | 6h | Docker 环境配置 |
| S005 | 6h | 数据管理 |

**合计**: 26h

---

## 任务详情文档

| 任务ID | 文档路径 |
|--------|----------|
| T001 | [T001-project-infrastructure.md](tasks/T001-project-infrastructure.md) |
| T002 | [T002-markdown-editor.md](tasks/T002-markdown-editor.md) |
| T004 | [T004-docker-environment.md](tasks/T004-docker-environment.md) |
| T010 | [T010-rnaseq-template.md](tasks/T010-rnaseq-template.md) |

---

*最后更新: 2026-03-30*