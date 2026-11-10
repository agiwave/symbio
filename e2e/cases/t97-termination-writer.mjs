// T97 终止写方：一轮算完 = 终答 ∧ 无未还债，落一条 `session.terminated`。
//
// ## 这条用例钉的是什么
//
// `docs/003-design/051-机制形状规格.md` §5 把「一个 scope 算完」定为**写方**的谓词，
// 但宿主此前只把投影交给 `DebtSource::rows`——簿记写方读不到环的事实投影。D-367
// 给了宿主侧登记面（`termination` 族）：每次 `drive` 前后交同一份只读投影 + 宿主的
// `NoDebt` 结论，写方据此落事实。
//
// 本用例证的是**端到端**那一跳：
//
// | 要证的事 | 读数 |
// |---|---|
// | 一轮正常收束后真的落终止事实 | WAL 里 `session.terminated` 恰好一条 |
// | 终止事实指向该轮的开轮事实 | `payload.root_id` = 该轮 `user.message` 的 `event_id` |
// | 同一 `root_id` 只落一次（前后两次观察幂等） | 两轮各一条，没有重复的 `root_id` |
//
// ## 为什么必须是两轮
//
// 只跑一轮只能证「落了」，证不了「幂等」：宿主在**每次驱动前后**观察，同一轮里
// 可能驱动多次（模型轮 + 工具轮）。两轮给出两个不同的 `root_id`，既能证明每轮
// 都覆盖，也能证明同一轮不会重复落。
//
// ## 事实源读数为什么可信
//
// WAL 是事实源本身（`session/<sid>/v2-events.wal`），不是投影。终止写方的草稿经
// 宿主写口落库，`(kind, key)` 吸收重复——这里读到的条数就是事实面的条数。
import {
  MockLlm,
  makeHomedir,
  dialogFaceOff,
  cleanupHomedir,
  runCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  readFileSyncSafe,
} from '../helpers.mjs';
import { join } from 'node:path';

/** 本用例的会话 id */
const SID = 'e2e-t97';

export default defineCase(
  'T97 终止写方：每轮收束后落一条 session.terminated，同 root 不重复',
  async () => {
    const llm = await new MockLlm([
      { id: 'final-1', match: '第一轮', content: '第一轮完成。' },
      { id: 'final-2', match: '第二轮', content: '第二轮完成。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      // 本用例只验终止事实，与对话面正交 ⇒ 三个开关钉死（理由见 `DIALOG_FACE_OFF`）。
      pluginConfigs: { ...dialogFaceOff() },
    });

    try {
      const first = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '第一轮',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(first.code, 0, `第一轮 CLI 退出码（stderr: ${first.stderr.slice(0, 600)}）`);

      const second = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '第二轮',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(second.code, 0, `第二轮 CLI 退出码（stderr: ${second.stderr.slice(0, 600)}）`);

      const events = readFileSyncSafe(
        join(hd.homedir, 'session', SID, 'v2-events.wal'),
      )
        .split('\n')
        .filter(Boolean)
        .map((line) => JSON.parse(line));

      const roots = events
        .filter((e) => e.kind === 'user.message')
        .map((e) => e.event_id);
      const terminated = events.filter((e) => e.kind === 'session.terminated');
      assertEq(
        terminated.length,
        roots.length,
        `每轮恰好一条终止事实（开轮 ${roots.length} 次，终止 ${terminated.length} 次）`,
      );
      const terminatedRoots = terminated.map((e) => e.payload?.root_id);
      assertEq(
        new Set(terminatedRoots).size,
        terminatedRoots.length,
        `同一 root_id 只能落一次（实际: ${JSON.stringify(terminatedRoots)}）`,
      );
      for (const root of roots) {
        assert(
          terminatedRoots.includes(root),
          `终止事实必须指向该轮的开轮事实（缺 ${root}：${JSON.stringify(terminatedRoots)}）`,
        );
      }
      // I3 到点必答：终止格也是「这一轮为什么停」的答案，`reason` 必须原样等于
      // 该轮终答事实上的那一格（唯一 owner 是写终答的那一行，这里不重算）。
      const finals = events.filter((e) => e.kind === 'chat.assistant.final');
      for (const t of terminated) {
        const root = t.payload?.root_id;
        const final = finals.find((e) => e.payload?.root_id === root);
        assert(final, `终止格 ${root} 找不到对应终答`);
        assert(
          t.payload?.reason,
          `终止格必须带 reason（root=${root}，实际: ${JSON.stringify(t.payload)}）`,
        );
        assertEq(
          t.payload?.reason,
          final.payload?.reason,
          `终止格的 reason 必须原样等于终答（root=${root}）`,
        );
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'CLI / mock-llm 子进程退出',
      });
    }
  },
);
