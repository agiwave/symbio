# 真实日志夹具

`real-session.tsv` 是一次**真实运行**的事件信封（不含载荷），给
[`invariants.rs`](../invariants.rs) 做「三条不变量在真数据上成立吗」的回放。

## 为什么要有它

验证器此前喂的是自己造的日志：`healthy_log()` 六条、`violated_log()` 十条。
那种数据能证明「检查器会响」，**证不了「跑起来的系统遵守不变量」**——
后者的前提是数据不是自己写的。第一次拿真日志回放就撞出两件事（见下「实测」），
这正是自造数据永远撞不出来的。

## 只留信封的理由

I1 / I2 / I3 判的是 `seq` / `turn` / `produced_by` / `cost_ms` / `status`。
载荷里有用户正文与工具输出，与本组判据无关——不搬进来是**划界**，不是裁剪事实。
（也因此这份夹具可以进仓：它不含任何对话内容。）

## 实测（回放当下就撞出来的两件事）

1. `chat.assistant.final` 的 `cost_ms = 893184`，而该轮 `user.message` 携带的
   `budget_ms = 60000`——差 14.9 倍。**不是违规，是口径错位**：
   `turn_runner.rs` 写进收束格的是**整轮累计**（含全部工具往返），
   而 `budget_ms` 是**单次模型调用**的事前闸门。用 `cost_ms > budget_ms` 判 I3，
   量的是两把不同的尺。
2. 于是 I3「到点必答」在信封上是**判不了**的：判定它需要「每次调用的耗时」，
   而日志里只有轮累计。这条缺口写进了 `invariants.rs` 的 I3 边界，不再假装判到了。

## 重新生成

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
