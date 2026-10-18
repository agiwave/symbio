---
kind: task
id: T-162-ClassifyOnly快速档路径裁决
title: 令牌闸门的快速档（ClassifyOnly / issue_fast）零生产调用——接线，或显式退役并从 F3 论证划界
status: todo
created: 2026-10-17
track: plan
prio: later
parent: T-156-架构机制化与旧代码清理批
where: symbio/src/symbio_core/adapters/mod.rs · symbio/src/plugins/classify/ · symbio/src/plugins/session/v2_exec.rs
exit: `ClassifyOnly` 令牌有生产调用路径（classify 走令牌闸门）**或**显式退役（成文 ADR 说明为什么快速档不做成令牌闸门、classify 插件为何是正确落点）；两种结论都必须让「快速档不得生成」这条约束有强制力，而不是靠 classify 插件「结构上不注册 Capability」这个巧合；cargo test + clippy + 门禁全绿
signal: `ClassifyOnly` 与 `TokenIssuer::issue_fast()`（adapters/mod.rs:99-115,153-154）只在 adapters/mod.test.rs:47,55 出现，**生产代码零调用**；对照组 `issue_reflex`（RuleOnly）与 `issue_deep`（FullModel）在 v2_exec.rs:1360,1376 有真实调用。实际的分类由独立插件 classify 承担（chat_loop/decide.rs:55 经容器路由 classify/decide），未走令牌闸门——「快速档只允许分类不允许生成」这条约束（PL-001 §10）今天靠的是 classify 插件不注册任何 Capability 这个结构巧合，不是 ClassifyOnly 令牌。adapters 域 README 已诚实标注 CanClassify/CanGenerate 是 `#[cfg(test)]`、无生产接入位
---

## 内容

父任务：[T-156-架构机制化与旧代码清理批](156-架构机制化与旧代码清理批.md)
依据：[PL-001 §10](../004-plan/001-核心架构.md "PL-001-核心架构")（快通道做的是**分类**不是生成；
「只允许分类」由令牌类型强制，**不靠 prompt 里写一句「请不要生成」**）· 判据 J3

**问题**：设计说「快速档 = 只允许分类不允许生成，由 `ClassifyOnly` 令牌类型强制」。
现实是：那个令牌**从未被签发**（`issue_fast()` 零生产调用），分类实际由 `classify` 插件承担，
而它**绕过了令牌闸门**。

这条与 [T-157-ActorSpec-pattern死字段接线](157-ActorSpec-pattern死字段接线.md) 是**同一个病的两面**：
F3 冻结锚点里的两个机制（`pattern` 三模式、四档令牌）都**有数据无行为**。

**但这里有一个需要诚实划界的点**：`classify` 插件**结构上不注册任何 Capability**
（`classify/plugin.rs:18-20`），所以它的工具集里**不可能出现**生成类工具——
「不得生成」这条约束**今天是成立的**，只是它的强制力来自**插件切分**，不来自**令牌类型**。

这正是 PL-012 §0.1.2 说的「切分就是架构的切分线」的一个正面案例：
**用切分实现的约束，比事后过滤更硬。** 所以「接线」未必是唯一正确答案。

## 两条合法出路（必须二选一，不能悬着）

| 出路 | 适用条件 | 代价 |
|---|---|---|
| **A 接线**：让 classify 经 `issue_fast` 拿令牌 | 若将来分类不只在 classify 插件发生（如 session 内的就地分类） | 需要生产接入位，且要证明令牌真的拦住了什么 |
| **B 显式退役**：承认快速档不做成令牌闸门 | 若「分类 = classify 插件」是稳定结构 | 需成文 ADR 从 F3 论证里划界，并把 `ClassifyOnly` / `issue_fast` / `CanClassify` 从契约面收掉 |

**不能接受的现状**：一个冻结契约名躺在 core 里、有测试守着它的编译期形状、
却没有任何生产路径——它是**假的机制感**（读代码的人以为有四档闸门，实际只有两档）。

## 处理过程

- **无论选哪条，都要让约束有强制力**：A 走令牌类型，B 走插件切分（成文并加一条 e2e 断言
  「classify 的工具集恒空」，把这个巧合变成判据）。
- **不要两边都留一半**：既不接线也不退役 = 现状，正是本条要消除的。
- 走 A 时注意：`CanClassify` / `CanGenerate` 今天是 `#[cfg(test)]`，升为生产项会**变宽 core**
  （CS-001 的 defaults/methods 格），须走 CS-002（`core-shape-allow` + 存在的 ADR）。

## 勘察结论（2026-10-19，暂缓）

**暂缓的理由**：本条与 [PL-014-单环一表收敛](../004-plan/014-单环一表收敛.md "PL-014-单环一表收敛")
的六个步骤**无依赖关系**（全仓 `grep T-162` 只命中父批与本看板），而两条出路的代价都落在
**冻结锚点 F3** 上。改架构要先定语义，不是先改文档——故本条停在「事实已核清、语义待定」。

核出的三条事实（下一轮从这里开始，不要重做）：

1. **那个端口从来没被建过**。`PL-005 §3.3` 的注入表写着快速档「注入的依赖 = `Classifier`」，
   而全仓 `grep Classifier` 在 `plugins/classify/` 之外**零命中**。`ClassifyOnly` 成孤儿，
   不是它没用，是它要守的端口不存在。`classify/decide.rs` 直接调 `ModelProvider::execute_turn`
   拿原始文本、自己 `parse_verdict`。
2. **文档里混着两个不同的快速档**，`ClassifyOnly` 只对其中一个成立：

   | | (i) 纯规则分类器 | (ii) 一次受限的模型调用 |
   |---|---|---|
   | 出处 | `verify/latency_gate.rs` 的 `Classifier`（`text.contains("？")`） | [PL-001 §10](../004-plan/001-核心架构.md "PL-001-核心架构") 的表：允许模型 = **是** |
   | 要不要令牌 | 不需要（不碰模型，`RuleOnly` 够） | 需要（允许调模型、不许长输出） |
   | 今天实现 | — | 就是这条 |

   所以这不是「架构设计不合理」，是**两句话各指一个快速档，而实现只落地了其中一半**。
3. **「不允许生成」这句措辞站不住**：若指「模型不产生 token」，那是 `max_tokens` 参数；
   若指「输出空间受限」，那要由端口的**返回类型**强制。三件事被压成了一句话。

**顺带核出一个比本条更大的缺口**（已单独成条：[T-168-模型档位闸门对内部请求无效](168-模型档位闸门对内部请求无效.md "T-168-模型档位闸门对内部请求无效")，本行 2026-10-19 订正）：闸门只装在
`LlmAdapter::generate(&FullModel, …)` 上，而 `ModelProvider::execute_turn` 是对所有插件开放的
core trait——**直调它就绕过闸门**。生产代码里有三处这样绕过（`tools: &[]` + `silent()` 出口的
内部请求）：`classify/decide.rs:146` · `compose/wording.rs:105` ·
`session/context/pipeline.rs::run_compression_llm`（第 977 行的调用）。
**session 的主链路不在此列**——`turn_exec.rs` 两处都先 `ProviderLlmAdapter::for_turn` 把
provider 包成 adapter 再走；它持 `Arc<dyn ModelProvider>` 只为每轮包一次。
（初版把 session 也算成绕过点，是「持 raw provider」与「直调 execute_turn」混为一谈，特此订正。）

**2026-10-19 更新**：T-168 已按「显式声明」结案（[ADR-080](../010-adr/080.md "ADR-080-内部请求显式脱离档位闸门")）——「内部请求
不受档位闸门约束」从没人说过变成书面裁决，并由 IM-004 把形状钉进门禁。**三处里 `compose` 与
压缩已各自归位**（判为内部请求：它们拿到的令牌本来就是 `FullModel`，接线是空转）；
`classify` 那处仍挂着等本条：该拿 `ClassifyOnly`，而 `LlmAdapter` 没有这个入口，补入口即 core
变宽（CS-002）——与本条 (ii) 分支是同一件事。故若本条裁 (i) 纯规则分类器，那次模型调用本身
就该消失，(ii) 的补入口一并作废；裁 (ii) 才需要另立一条专门记入口的形状与 core 面代价。

**下一步（定语义再动代码）**：先裁决快速档是 (i) 还是 (ii)。若 (ii)，则补 `Classifier` 端口
（收 `&ClassifyOnly`、形参无 `tools`、`max_tokens` 取档案表的小值）——不带工具由类型强制、
不长输出由参数强制、只认四选一由解析强制，三件事各归其位，F3 不动。
