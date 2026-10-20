# 验证器夹具

两件东西，两种来历，都是「程序不该自己造的数据」：

| 文件 | 谁读 | 来历 |
|---|---|---|
| [`real-session.tsv`](real-session.tsv) | [`invariants.rs`](../invariants.rs) | 一次**真实运行**的事件信封——人不该自己编 |
| [`paraphrases.tsv`](paraphrases.tsv) | [`recall_eval.rs`](../recall_eval.rs) | 33 条查询的**同义换述**——答案不该由出题人抄进题目 |

## `real-session.tsv`：真实日志夹具

`real-session.tsv` 是一次**真实运行**的事件信封（不含载荷），给
[`invariants.rs`](../invariants.rs) 做「三条不变量在真数据上成立吗」的回放。

### 为什么要有它

验证器此前喂的是自己造的日志：`healthy_log()` 六条、`violated_log()` 十条。
那种数据能证明「检查器会响」，**证不了「跑起来的系统遵守不变量」**——
后者的前提是数据不是自己写的。第一次拿真日志回放就撞出两件事（见下「实测」），
这正是自造数据永远撞不出来的。

### 只留信封的理由

I1 / I2 / I3 判的是 `seq` / `turn` / `produced_by` / `cost_ms` / `status`。
载荷里有用户正文与工具输出，与本组判据无关——不搬进来是**划界**，不是裁剪事实。
（也因此这份夹具可以进仓：它不含任何对话内容。）

### 实测（回放当下就撞出来的两件事）

1. `chat.assistant.final` 的 `cost_ms = 893184`，而该轮 `user.message` 携带的
   `budget_ms = 60000`——差 14.9 倍。**不是违规，是口径错位**：
   `turn_runner.rs` 写进收束格的是**整轮累计**（含全部工具往返），
   而 `budget_ms` 是**单次模型调用**的事前闸门。用 `cost_ms > budget_ms` 判 I3，
   量的是两把不同的尺。
2. 于是 I3「到点必答」在信封上是**判不了**的：判定它需要「每次调用的耗时」，
   而日志里只有轮累计。这条缺口写进了 `invariants.rs` 的 I3 边界，不再假装判到了。

### 重新生成

WAL 变了（新会话）就重跑下面这段，**不要手改 tsv**：

```bash
node -e "
const fs=require('fs');
const evs=fs.readFileSync('.symbio/session/<id>/v2-events.wal','utf8')
  .split(/\r?\n/).filter(Boolean).map(l=>JSON.parse(l));
const rows=evs.map(e=>{const p=e.payload||{};return [
  e.seq,e.kind,e.turn,e.actor,e.produced_by==null?'-':e.produced_by,
  e.cost_ms,p.status||'-',p.fell_back===true?'1':'0'].join('\t')});
fs.writeFileSync('docs/004-plan/verify/fixtures/real-session.tsv',
  ['# 见同目录 README.md',...rows,'' ].join('\n'));
"
```

`<id>` 取 `.symbio/session/` 下最新的那个目录；表头（前 9 行注释）按现状保留即可。

---

## `paraphrases.tsv`：换述集（T-173）

33 条用例各 ≥2 条同义换述，给 [`recall_eval.rs`](../recall_eval.rs) 出第四个数字：
同一批池、只把查询换个说法，`dependency = 原述 recall@k − 换述 recall@k`。
它是 [PL-002 §3.1.1](../../002-能力坐标系.md "PL-002-能力坐标系")「抽象层级」格的
离线效标，此前那一格写的是「本轴目前只能人工判」。

### 格式

`用例名<TAB>换述`，一行一条；用例名必须**逐字等于** `recall_eval.rs` 里 `Case::name`
（名字是两边唯一的关联键，写错即静默丢一条——所以程序会 panic 而不是跳过）。
`#` 与空行忽略，**不许加第三列**。

### 两条写换述的纪律（违反则读数变成一个好看的 A）

1. **写换述的人不许看记忆正文**。查询是「用户想找什么」，正文是「答案」。
   照正文里的词写查询 = 把答案抄进题目。
2. **尽量避开原查询的二元组**。`recall_score` 的算法就是二元组重叠：换述若与原述
   共享大量二元组，「依赖字面」根本测不出来，测到的只是「换了同义词还认得」。
   程序会把「与原述的重叠均值」打出来（当前 **0.121**），就是给这条纪律一个读数。

### 冻结与指纹

换述是人写的 ⇒ 它只是效标的**下界**：写得越接近原述，差值越小越好看。所以这份表
一次写完即**冻结**，改它必须同时做两件事，缺一条程序就红：

1. 改 `recall_eval.rs` 的 `PARAPHRASES_FINGERPRINT`（FNV-1a 64，按 `\r\n → \n` 归一后算）；
2. 在下面的**改动记录**里写为什么改。

理由写在**这里**而不是提交信息里：提交信息会随历史沉底，而这个文件是下一次改它的人
真正会打开的地方。

### 读数不覆盖什么

- 它测的是**排序器对措辞的敏感度**，不是「系统会归纳」。低读数只说明这套排序不靠
  字面，不说明任何东西理解了同义。
- 有 6 条用例（时间兜底类：期望的就是最新那条）**结构上**与措辞无关——任何查询都
  按时间兜底命中，换述读数在上面恒为 1。所以程序同时打两个数：全 33 案 与
  「措辞能左右结果」的 27 案。后者才是能引用的那个。
- 5 条用例的换述未全中（`crashloopbackoff-diagnosis` / `flaky-unit-test` /
  `mobile-upload-fail` / `loadtest-vs-prod` / `dashboard-empty`），其中
  `mobile-upload-fail` 两条全不中——换述说的是同一件事，但二元组与正文没有交集
  （`k8s / 容器 / 反复重启` vs 正文里的 `Kubernetes / pod / CrashLoopBackOff`）。
  那不是题写坏了，**那正是读数**。

### 改动记录

这张表是**冻结机制的一部分**，所以它必须活着、且必须在这个文件里：改换述的人打开的是
这个文件，不是 `git log`。表的语义只有一条——**每一行是一次有理由的改动**；
没有理由的改动不该出现（那正是指纹锁要拦的东西）。

| 日期 | 改动 | 为什么 |
|---|---|---|
| 2026-10-20 | 建集：33 案 × 2 条 = 66 条，指纹 `0x5fe9a03f3f961ae5` | 建集即冻结；写换述时只看 query 不看 `memories`（纪律①），术语按用户口吻换（纪律②） |

