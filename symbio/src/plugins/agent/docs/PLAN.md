# Symbio Agent：执行计划

> **定位**：项目**未来工作**聚焦文档——只关注"我们要做什么"。
> 历史成果见 [CHANGELOG.md](./CHANGELOG.md)；当前问题见 [ISSUES.md](./ISSUES.md)。

---

## 1. 当前状态

| 维度 | 状态 |
|------|------|
| 核心能力 | 3 个：agent_chat + agent_cognition（统一认知，5 操作）+ agent_create |
| 架构 | 单向依赖，0 循环依赖，10 个模块 |
| 单元测试 | 全部通过（以 `cargo test --lib plugins::agent` 实时输出为准） |
| Clippy | 0 warnings |
| 活跃问题 | 0 个（ISSUES.md） |
| 目录结构 | v38 优化后：消除所有 factory.rs，构建逻辑内聚到 plugin.rs |

### 1.1 当前能力体系

| 能力 | 说明 | 操作 |
|------|------|------|
| agent_chat | 对话能力 | - |
| agent_cognition | **统一认知工具** | 5 个操作（memory 域） |
| agent_create | 创建智能体 | - |

**`agent_cognition` 操作列表**（`operation: "域.操作"`）：

| 域 | 操作 | 文件 |
|------|------|------|
| memory | save, retrieve, graph_query, reflect, consolidate | ops/memory/*.rs |

> 每个操作是独立的 `.rs` 文件，实现 `CognitionOp` trait，通过 `OpRegistry` 注册表统一分发。
> 认知判断（推理、规划、元认知等）由 LLM 主导，模块仅提供底层 CRUD 和数据管理能力。

---

## 2. 待办任务

### 2.1 短期（S — 1-2 周）

| 编号 | 任务 | 说明 | 优先级 |
|------|------|------|--------|
| S-1 | ~~引擎层单元测试补齐~~ | ~~用 mock 给 scaffold 写 10+ 测试~~ | ✅ v38 已完成（+17 测试，总计 23 个） |
| S-2 | ~~Token 优化 + 语义化~~ | ~~移除冗余参数信息、修复工具名引用、语义化子操作名称~~ | ✅ v43 已完成（节省约 1000-1500 tokens/请求） |
| S-3 | E2E 测试 CI 化 | 拆"无 LLM"（CI 必跑）+ "含 LLM"（nightly） | P2 |
| S-4 | 冷启动回归测试 | 桶索引为空时 search 行为不退化 | P3 |
| S-5 | CAS 锁粒度基准 | per-id 锁 vs 全局锁的吞吐对比 | P3 |
| S-6 | shutdown 时长基准 | cancel 后 rebuild 在 100ms 内退出 | P3 |

### 2.2 中期（M — 1 个月）

| 编号 | 任务 | 说明 | 优先级 |
|------|------|------|--------|
| M-1 | HNSW ANN 接入 | 替换桶索引近邻扫描，提升检索性能 | P2 |
| M-2 | 认知工具语义化改造 | `search_evidence`/`detect_conflicts` 改用 embedding 语义搜索替代子串匹配；`plan.decompose` 重新定位（分解是 LLM 职责，工具层只存储） | P2 |
| M-3 | `SqliteStorage::search` 实现 | FTS5 + FilterExpr SQL 化 | P2 |
| M-4 | 统一可观测性接入 | `MetricsSink` trait 已就绪，需接入 tracing / Prometheus | P2 |
| M-5 | insert 路径异步 embed | `EmbeddingQueue` (mpsc) + 后台 embedder | P2 |
| M-6 | `repair_bucket_index` 双 API | 拆同步/异步双 API（短期 inline 优化已完成） | P3 |

### 2.3 长期（L — 季度级）

| 编号 | 任务 | 说明 |
|------|------|------|
| L-1 | 多智能体协作 | 通过 agent_create + agent_chat 实现，无需额外 trait |
| L-2 | 知识图谱 ANN 混合检索 | 在 M-1 基础上 |
| L-3 | 推理引擎增强 | 策略选择器、置信度校准器 |
| L-4 | 类型系统演化 | `CognitiveUnit` 走 Serde derive 强类型 + Schema registry |
| L-5 | CAS 与持久化解耦 | CAS 仅校验版本号，持久化异步批量落盘 |
| L-6 | embed 服务本地化 | ONNX / Candle，消除远端依赖（已部分完成：FastEmbed 本地模型） |
| L-7 | 指标导出 | Prometheus / OpenTelemetry（M-4 基础上） |

---

## 3. 质量目标

| 指标 | 当前值 | 目标值 |
|------|--------|--------|
| 单元测试 | 195 | ≥300 |
| 测试覆盖率 | 核心能力 100% | ≥90% |
| 循环依赖数 | 0 | 0 |
| 能力数量 | 3 | ≤5 |
| Clippy warnings | 0 (除 MSRV in 1.91 const 提示) | 0 |

---

## 4. 深入阅读

- [**CHANGELOG.md**](./CHANGELOG.md)：历史修复记录（v25-v48）
- [**ISSUES.md**](./ISSUES.md)：当前活跃问题（0 个）
- [**ARCHITECTURE.md**](./ARCHITECTURE.md)：系统架构与模块设计
- [**PRINCIPLES.md**](./PRINCIPLES.md)：架构原则与质量标准
- [**COGNITION.md**](./COGNITION.md)：认知单元数据规范
- [**TESTING.md**](./TESTING.md)：自动化测试体系
