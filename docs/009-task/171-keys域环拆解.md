---
kind: task
id: T-171-keys域环拆解
title: keys 域环拆解——把「万能胶」从依赖图里拿掉
status: todo
created: 2026-10-20
track: plan
prio: later
deps: [T-165-控制面收敛]
where: symbio/src/symbio_core/keys/ · symbio/src/symbio_core/plugin/error.rs · symbio/src/symbio_core/core-vocab 审计基线
exit: `node scripts/core-vocab-audit.mjs` 的依赖环族从 **2 降到 1**（`capability⇄chat⇄event_bus⇄exec⇄keys⇄llm⇄plugin⇄present⇄vdfs` 这一族消失，`adapters⇄projection` 不动），且 V-002 基线同步下调、core 三格与根出口宽度不升；`keys` 域对上层域的引用数 = 0；每步一个提交、全量门禁与 e2e 全绿
---

## 内容

父设计：[PL-005 模块架构](../004-plan/005-模块架构.md "PL-005-模块架构")（一域一目录、域内私有、依赖单向）·
[PL-001 §8](../004-plan/001-核心架构.md "PL-001-核心架构")（取值准入）

**这是 core 里唯一的结构性欠账。** 形状纪律守得很好（`CS-001` 三格只降不升、
`V-001` 目录不存在），但依赖图上有两族环，其中一族是**八域连环**：

```
capability ⇄ chat ⇄ event_bus ⇄ exec ⇄ keys ⇄ llm ⇄ plugin ⇄ present ⇄ vdfs
```

`core-vocab-audit`（`V-002`）把基线记成「2 族，只许降」，于是这条环一直在，
只是**不许变多**。代价不是「编译慢」，是 README 里那句「依赖单向、只依赖更底层」
**事实上不成立**：改这八个域里的任何一个，都可能波及另外七个。任何「把某域往下
沉一层」的重构都要先回答「会不会踩到环」，而答案取决于当时的边，不是设计。

## 两条根因（各自独立，可分别处置）

### 根因一：键值类型直接指向各域类型

`keys` 域的键要能装 `Arc<dyn Capability>`、`Arc<dyn Registry>` 这类各域才认识的东西，
于是 `keys → capability` / `keys → plugin` 成为**硬边**；而那些域又在别处需要
`keys` 的键常量（`MODE` / `RISK_LEVEL` / `WORKDIR`），反向边同时成立。

拆法（不改语义）：键通道**值类型擦除**——`set`/`get` 走 `Arc<dyn Any + Send + Sync>`
加一次 `downcast`，或者按「键类型自带值类型」的关联类型做，让 `keys` 只认识
`Any` 与自己的 `Key` trait，不认识任何一个具体的域类型。取用方本来就知道自己
要什么类型，`downcast` 失败是编程错误，用 `expect`-free 的 `Option` 返回即可。

### 根因二：`PluginError::Aborted` 带半截 `ChatNode`

错误类型是**最小的公共词汇**，它引用 `chat::ChatNode` 就把 `plugin → chat` 变成硬边，
而 `chat` 又要 `plugin` 的 trait 才能跑——环的最短一条路径就是这两条边。

拆法：载荷改成一个**不透明句柄**（`Arc<dyn Any + Send + Sync>`）或独立的
`AbortPayload` 小类型，构造与解读都留在 `chat` 侧（谁产生谁解读，错误类型只搬箱子）。

## 处理过程

**为什么单独立批**（三条，按强度排）：

1. **改动面横跨 8 个域**，与任何在飞批次都会冲突；混在别的批里回退不干净。
2. 收益不体现在功能上（用户看不见），只有形状指标会动——容易被「顺手一起改」稀释成
   半成品，而半拆的环比整拆更难收。
3. 拆之前要先定**「谁拥有键通道」**这条裁决：类型擦除把安全从编译期挪到运行期
   （`downcast` 失败），这是用「一个域少认几种类型」换的。这条必须写进 ADR，
   否则下一个人会以「类型安全」为由把它改回去。

**施工序**（依赖序，不是价值序，见 [ADR-090](../010-adr/090.md "ADR-090")）：

1. 立 ADR：键通道值类型擦除 + `Aborted` 载荷去 `ChatNode` 化，写清代价与失效边界；
2. 根因二先做（边数少、影响面小，且是环的最短路径）；
3. 根因一按「引用数最少的键类型」逐个迁移，每迁一类跑一次 `core-vocab-audit`
   看环族数是否下降；
4. 环族消失后，把 `V-002` 基线从 2 改成 1，并把 `core/README.md` 里「依赖单向」
   那句从**声称**改为**可复核的读数指引**（指向审计脚本，不抄数字）。

