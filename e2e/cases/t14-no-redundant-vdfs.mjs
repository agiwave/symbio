import './_selfrun.mjs';
// T14 会话期间**零回读**：后端下发的变更必须自带载荷，消费端不该回读任何东西。
//
// 为什么这条性质需要 e2e 而不是单测：它跨了两端——**后端发什么**与**消费端怎么用**。
// 单测各锁一半（后端锁帧形状，前端锁「给定帧怎么落地」），只有真跑一轮才能证明
// 「合起来不需要额外请求」。
//
// 被锁的是三类回读，各自对应一种「载荷不够用」：
// - `vdfs/stat`：会话节点变更没带节点视图 ⇒ 消费端只能回读运行态；
// - `vdfs/read`：消息变更没带正文 ⇒ 消费端只能回读整条消息；
// - `vdfs/list`：目录条目变更没带清单 ⇒ 消费端只能重列目录。
//
// 判据用 CLI 的路由留痕（`SYMBIO_ROUTE_LOG=1`，见 `cli/src/client.rs`）。CLI 与
// Tauri 前端消费的是**同一套**变更通道与同一份信封语义（进程内路由 vs IPC，
// 信封形状相同），所以这里量到的「一轮会话需要多少次回读」对两端都成立。
//
// 反面证据：这条用例写出来之前，会话期间实测出现过成对的 `vdfs/stat` + `vdfs/read`
// 与成串的 `vdfs/list`（目录重拉被 400 ms 防抖放大）。根因不是通道，是 S27 把实时面
// 搬进通用 vdfs 频道时**没有重新界定该频道上各类帧的语义边界**。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

/** 从 stderr 里解出路由留痕序列（`[route] <path>`） */
function routeTrace(stderr) {
  const out = [];
  for (const line of stderr.split('\n')) {
    const m = line.match(/\[route\]\s+(\S+)/);
    if (m) out.push(m[1]);
  }
  return out;
}

const count = (routes, path) => routes.filter((p) => p === path).length;

export default defineCase('T14 会话期间零回读（无 stat / read / list）', async () => {
  const llm = await new MockLlm([
    { id: 'text', match: '你好', chunks: ['你好', '，', 'mock', '世', '界'], chunkDelayMs: 5 },
  ]).start();
  const hd = makeHomedir({ providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }] });
  try {
    const r = runCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      message: '你好',
      provider: PROVIDER_ID,
      session: 'e2e-t14',
      env: { SYMBIO_ROUTE_LOG: '1' },
    });
    assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
    // 先把「这一轮真的跑完了」钉住：否则任何一处提前失败都会让下面的零计数**空过**。
    assertEq(r.stdout.trim(), '你好，mock世界', 'stdout 应为分片按序拼接的完整正文');

    const routes = routeTrace(r.stderr);
    // 留痕本身要有内容——路由开关失效时不能退化成「零回读」的假绿灯。
    assert(count(routes, 'session/chat/send') === 1, `应恰好发出一次会话请求（实际留痕: ${routes.join(', ')}）`);

    assertEq(count(routes, 'vdfs/stat'), 0, '会话节点变更必须自带节点视图，不该回读 stat');
    assertEq(count(routes, 'vdfs/read'), 0, '消息变更必须自带正文，不该回读 read');
    assertEq(count(routes, 'vdfs/list'), 0, '本轮没有任何目录条目变化，不该出现 list');
  } finally {
    llm.stop();
    cleanupHomedir(hd);
  }
});
