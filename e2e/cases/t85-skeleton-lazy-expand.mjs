import './_selfrun.mjs';
// T85 折叠骨架的按需重取（`expand_skeleton`）端到端（DE-019 §6）
//
// ## 这条用例补的是什么
//
// 前端把折叠骨架的子节点**本地摘掉**（惰性重取），展开时经 `expand_skeleton`
// 按骨架 id 把那棵子树拉回来。这条链有一处只有端到端能证：**重取回来的那批窄帧，
// 必须与「全量读转写」里同 id 的节点逐字段一致**（`id` / `parent_id` / `seq`）。
//
// 后端单测（`vdfs_provider.test.rs`）造的是手工 `seed_nodes` 的树，前端单测
// （`skeletonLazy.spec.ts`）造的是手工扁平表——两侧各自自洽，谁都看不出
// 「后端按根算子 BFS」与「前端按 `parent_id` 组装」是不是同一棵子树。真会话里
// 跑一遍，两份才对得上；对不上时前端会重复渲染或丢内容，且只在长会话里现形。
//
// ## D-335 之后的形状（不是退役）
//
// 「整轮骨架 ⊃ 工具骨架 ⊃ 工具结果」那两层已不存在：整轮骨架按 D-335 并入
// **用户消息节点**（同 id 合并，读出的是 `body` 不是 `skeleton`）。剩下的折叠
// 主体是**工具卡**（`skeleton` + `style_tag=tool`），它仍有可展开的后代——工具
// 结果（一层）。所以能力还在，只是从两层变一层；本用例断言的是**懒展开确实
// 发生**：
//
// - 折叠视图（全量读去掉后代，即前端本地释放后的那份）里只有骨架本身，且骨架
//   自带的 `content` 只有调用参数，不含工具结果正文；
// - 结果正文**只**藏在后代里，唯一取到它的路是 `expand_skeleton`；
// - 重取回来合并后，折叠视图与全量读逐条相等。
//
// 若哪天工具卡也不再有后代，本用例会在「至少一个后代 / 结果只在后代里」这一条
// 上红——那意味着懒展开真的没有主体了，该按退役处理，而不是把判据改弱。
//
// ## 判据怎么取
//
// 1. **懒展开确实发生**：折叠视图里有骨架、无后代内容；`expand_skeleton` 取回的
//    正是被摘掉的那批（含结果正文），合并后与全量读一致。
// 2. **depth="1" 只直接子**：工具卡当前只有一层，故与 `all` 同集——判据是
//    「只回直接子」，不是「必须比 all 少」（D-335 之前才靠层数分道）。
// 3. **since_seq 增量**：只回 seq 更高的那批（窄帧序列的增量写法）。
// 4. **非法入参有错误码**（判据 1）：`depth` 给了 `"2"`、`since_seq` 给了字符串
//    ⇒ 被拒，**不静默按缺省返回整棵**——静默的后果是「以为只要直接子」的调用方
//    拿到整棵树，而他从回执里看不出来。
// 5. **只读**：重取前后全量转写一条不少（它是读出口，不是写动作）。
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  startLongLivedCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  DIALOG_FACE_OFF,
  waitFor,
  nextPort,
  readSessionMessages,
  sessionAddr,
  SEG_MESSAGES,
} from '../helpers.mjs';

/** gateway 入站配置（与其它用例同款：本机回环、无 token、http） */
function gatewayConfig(port) {
  return {
    inbound_enabled: true,
    inbound_protocol: 'http',
    inbound_bind: '127.0.0.1',
    inbound_port: port,
    inbound_token: '',
    inbound_readonly: false,
  };
}

const SID = 'e2e-t85';
const Q = '读一下工作区的 README';
const WS_MARKER = 'T85-WORKSPACE-MARKER：这一行是工具结果里的靶子。';

/** 三元组指纹：`id` / `parent_id` / `seq` —— 重取与全量读只对这三个字段对齐 */
function triple(m) {
  return `${m?.id}|${m?.parent_id ?? ''}|${m?.seq ?? ''}`;
}

/** 从扁平转写里按 `parent_id` BFS 出一棵骨架的**全部后代**（前端释放时摘的就是它） */
function descendantsOf(messages, rootId) {
  const kids = new Map();
  for (const m of messages) {
    const p = m?.parent_id;
    if (!p) continue;
    if (!kids.has(p)) kids.set(p, []);
    kids.get(p).push(m);
  }
  const out = [];
  const queue = [rootId];
  const seen = new Set([rootId]);
  while (queue.length) {
    const cur = queue.shift();
    for (const k of kids.get(cur) ?? []) {
      if (seen.has(k.id)) continue;
      seen.add(k.id);
      out.push(k);
      queue.push(k.id);
    }
  }
  return out;
}

/** 动作回执里的 `nodes`（`VdfsActionResult.data.nodes`） */
function nodesOf(resp) {
  return resp?.body?.data?.data?.nodes ?? null;
}

/**
 * 非法入参是否被**拒绝**。
 *
 * 判据是「回执不是一次成功动作」：HTTP 非 200，或 200 但不带成功的动作回执。
 * 关键是**不能出现 `nodes`**——一旦带回来，调用方就会把「整棵子树」当成
 * 「我只要的直接子」，而这正是判据 1 要挡的静默退化。
 */
function rejected(resp) {
  const ok = resp?.status === 200 && resp?.body?.data?.ok === true;
  return !ok;
}

export default defineCase(
  'T85 折叠骨架的按需重取：折叠视图里只有骨架、结果正文只在展开后取到，非法入参有错误码',
  async () => {
    const llm = await new MockLlm([
      {
        id: 'a1-tool',
        match: Q,
        once: true,
        toolCalls: [{ id: 'c_a1', name: 'vdfs_read', arguments: { path: 'README.md' } }],
      },
      { id: 'a1-final', afterTool: true, once: true, content: '读完了。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: gatewayConfig(GATEWAY_PORT),
        session: { ...DIALOG_FACE_OFF },
      },
    });
    writeFileSync(join(hd.workdir, 'README.md'), `${WS_MARKER}\n`);

    try {
      const r = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        session: SID,
        stdinText: `${Q}\n`,
      });
      assertEq(r.code, 0, `首轮退出码（stderr: ${r.stderr.slice(0, 600)}）`);

      const cli = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await cli.waitGatewayReady();

        // ── 全量读：前端加载历史用的就是这一条（`<根>/session/<sid>`）────────
        const all = await readSessionMessages(cli, SID);
        assert(Array.isArray(all) && all.length > 0, `应读到转写（实际: ${JSON.stringify(all)?.slice(0, 300)}）`);

        // ── 骨架仍在，但折叠主体只剩工具卡（D-335 把整轮骨架并入用户消息）──
        const skeletons = all.filter((m) => m?.type === 'skeleton');
        assert(skeletons.length > 0, `转写里应有骨架（实际类型: ${all.map((m) => m?.type).join(',')}）`);
        assert(
          skeletons.every((s) => (s.meta?.style_tag ?? '') !== 'run'),
          `整轮骨架按 D-335 并入用户消息节点，不应再有 run 骨架（实际: ${JSON.stringify(
            skeletons.map((s) => [s.id, s.meta?.style_tag ?? null]),
          )}）`,
        );
        const target = skeletons
          .map((s) => ({ s, kids: descendantsOf(all, s.id) }))
          .sort((a, b) => b.kids.length - a.kids.length)[0];
        // 「还有没有可展开的对象」的判据就落在这里：工具卡仍有后代（工具结果）。
        // 若哪天为 0，懒展开已无主体——该整例退役，而不是把这句话删掉。
        assert(
          target.kids.length >= 1,
          `工具卡骨架应仍有可展开的后代（实际 0 个：懒展开已无主体，应按退役处理）`,
        );

        const addr = await sessionAddr(cli, SID);
        const targetPath = `${addr}/${SEG_MESSAGES}/${target.s.id}`;

        /** 发一次 `expand_skeleton` */
        const expand = (payload) =>
          cli.invoke(
            'vdfs/action',
            { path: targetPath, action: 'expand_skeleton', payload },
            { session_id: SID },
          );

        // ── 判据 1：懒展开确实发生（不是只数节点）──────────────────────────
        //
        // 前端释放 = 把这棵子树从本地列表里摘掉。折叠视图必须只剩骨架本身：
        // 有骨架、没有后代；且骨架自己的 `content` 只有调用参数——工具结果
        // 正文**只在**后代节点里。于是「结果正文能出现」这件事只能由展开完成。
        const kidIds = new Set(target.kids.map((k) => k.id));
        const collapsed = all.filter((m) => !kidIds.has(m.id));
        assert(collapsed.some((m) => m.id === target.s.id), '折叠视图里应有骨架本身');
        assert(
          !String(target.s.content ?? '').includes(WS_MARKER),
          `骨架的 content 只该有调用参数：结果正文若也在骨架里，懒展开就是空跑（实际: ${JSON.stringify(
            target.s.content,
          )?.slice(0, 200)}）`,
        );
        assert(
          target.kids.some((k) => String(k.content ?? '').includes(WS_MARKER)),
          '工具结果正文应藏在后代里（折叠时看不见）',
        );
        assert(
          collapsed.every((m) => !String(m.content ?? '').includes(WS_MARKER)),
          '折叠视图里不应出现工具结果正文（否则前端不必展开）',
        );

        const respAll = await expand({ depth: 'all' });
        assertEq(
          respAll.status,
          200,
          `expand_skeleton 应受理（${JSON.stringify(respAll.body)?.slice(0, 400)}）`,
        );
        // 对照锚：成功回执必须是 `ok: true`——否则下面「被拒 = 不是成功回执」
        // 这一判据会因成功与失败同形而空转（两条都判成拒绝）。
        assertEq(
          respAll.body?.data?.ok,
          true,
          `成功的动作回执应带 ok=true（${JSON.stringify(respAll.body)?.slice(0, 400)}）`,
        );
        const gotAll = nodesOf(respAll);
        assert(Array.isArray(gotAll), `回执应带 nodes 数组（实际: ${JSON.stringify(respAll.body)?.slice(0, 400)}）`);
        assertEq(
          gotAll.map(triple).sort(),
          target.kids.map(triple).sort(),
          '重取的子树须与全量读里的后代逐条一致（id / parent_id / seq）',
        );
        assert(
          gotAll.some((n) => String(n.content ?? '').includes(WS_MARKER)),
          '重取应带回工具结果正文（不只是 id）',
        );
        const merged = [...collapsed, ...gotAll];
        assertEq(
          merged.map(triple).sort(),
          all.map(triple).sort(),
          '重取合并回折叠视图后，应与全量读逐条相等',
        );

        // ── 判据 2：depth="1" 只回直接子 ────────────────────────────────────
        //
        // D-335 之后工具卡只有一层，所以它与 `all` 同集——判据是「只回直接子」，
        // 不是「必须比 all 少」。层数分道是整轮骨架还在时的事。
        const respOne = await expand({ depth: '1' });
        assertEq(respOne.status, 200, `depth="1" 应受理（${JSON.stringify(respOne.body)?.slice(0, 400)}）`);
        const gotOne = nodesOf(respOne);
        assert(Array.isArray(gotOne), 'depth="1" 回执应带 nodes 数组');
        assert(
          gotOne.length > 0 && gotOne.every((n) => n.parent_id === target.s.id),
          `depth="1" 只该有直接子（实际: ${JSON.stringify(gotOne.map((n) => [n.id, n.parent_id]))}）`,
        );
        assertEq(
          gotOne.map(triple).sort(),
          target.kids.filter((k) => k.parent_id === target.s.id).map(triple).sort(),
          'depth="1" 的直接子也须与全量读一致',
        );

        // ── 判据 3：since_seq 只回更新的那批 ────────────────────────────────
        const seqs = target.kids.map((k) => k.seq).filter((s) => typeof s === 'number');
        assert(seqs.length === target.kids.length, '后代都应带 seq（顺序的唯一权威锚点）');
        const water = Math.min(...seqs);
        assert(water > 0, 'seq 从 1 起发号（since_seq=0 才是「不过滤」）');
        const respSince = await expand({ since_seq: water });
        assertEq(respSince.status, 200, `since_seq 应受理（${JSON.stringify(respSince.body)?.slice(0, 400)}）`);
        const gotSince = nodesOf(respSince);
        assertEq(
          gotSince.map(triple).sort(),
          target.kids.filter((k) => k.seq > water).map(triple).sort(),
          `since_seq=${water} 只该回 seq 更高的后代`,
        );
        // 反向锚：`since_seq=0` 时全部后代都在增量里——否则「只回更高」是空跑。
        const respSince0 = await expand({ since_seq: 0 });
        assertEq(
          (nodesOf(respSince0) ?? []).map(triple).sort(),
          target.kids.map(triple).sort(),
          'since_seq=0 应回全部后代',
        );

        // ── 判据 4：非法入参被拒，不静默按缺省 ───────────────────────────────
        for (const [payload, why] of [
          [{ depth: '2' }, 'depth 只接受 "1" | "all"'],
          [{ depth: 1 }, 'depth 必须是字符串'],
          [{ since_seq: '3' }, 'since_seq 必须是整数'],
        ]) {
          const resp = await expand(payload);
          assert(
            rejected(resp),
            `${why} ⇒ 应被拒（status ${resp.status}，body ${JSON.stringify(resp.body)?.slice(0, 300)}）`,
          );
          assert(
            nodesOf(resp) === null,
            `${why} ⇒ 不得带回 nodes（静默按缺省就会带，实际: ${JSON.stringify(nodesOf(resp))?.slice(0, 200)}）`,
          );
        }

        // 合法边界照旧放行：显式 null = 缺省（「不过滤」的线上写法）
        const respNull = await expand({ depth: null, since_seq: null });
        assertEq(
          respNull.status,
          200,
          `显式 null 应照旧受理（${JSON.stringify(respNull.body)?.slice(0, 300)}）`,
        );
        assertEq(
          (nodesOf(respNull) ?? []).map(triple).sort(),
          target.kids.map(triple).sort(),
          '显式 null = 缺省：仍是整棵子树',
        );

        // ── 判据 5：只读——重取前后转写一条不少 ──────────────────────────────
        const after = await readSessionMessages(cli, SID);
        assertEq(after.length, all.length, 'expand_skeleton 是读出口：转写条数不得变化');
        assertEq(
          after.map(triple).sort(),
          all.map(triple).sort(),
          'expand_skeleton 是读出口：转写内容不得变化',
        );
      } finally {
        cli.stop();
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
