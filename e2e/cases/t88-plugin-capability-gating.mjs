import './_selfrun.mjs';
// T88 插件接入即能力：装配位（`PLUGIN.yml` 的 `plugin_enabled`）决定能力有无。
//
// ## 这条用例补的是什么
//
// 「能力的扩充不改内核」已由结构钉住（插件身份与能力族都归声明方，DE-021 D-174 / D-175）；
// 但「接入才有能力」此前只有构造期单测（`PluginRegistry::mount_all` 跳过停用目录），
// **没有一条用户真的会走的链路**证明：停用一个插件后，它的工具从模型工具面消失，
// 而会话照常跑完、装载不红。本用例就取这两个读数。
//
// ## 判据怎么取
//
// 同一套夹具、同一条用户消息，只翻 `local` 的装配位：
// - 启用：模型请求的 `tools` 里应出现 `local` 自己的工具（`content_search` 等）；
// - 停用：同一组名字一个都不该出现，且 CLI 仍以 0 退出、收尾正文照常落 stdout。
//
// 「启用时出现」不是多余的：只验停用侧的话，工具面整体为空也会绿——那证明不了
// 读数是装配位造成的。两侧成对取，才是「接入 ⇒ 有能力 / 不接入 ⇒ 无能力」。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  runCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

/** `local` 插件的四个工具名（其余插件的工具同名概率为零，足以当装配位的读数）。 */
const LOCAL_TOOLS = ['content_search', 'codebase_search', 'ask_user', 'todo_write'];

/**
 * 跑一轮会话，取**首次模型请求**里的工具名。
 *
 * 只取第 0 次请求：工具面在收集期装配，后续工具结果轮的列表是同一次收集的副本；
 * 取最后一次只是把同一事实再读一遍，且会把「工具轮里的增量」误当成装配结果。
 */
async function toolNamesFor({ localEnabled, sessionTag }) {
  const llm = await new MockLlm([{ id: 'final', content: '收到。' }]).start();
  const hd = makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
    pluginConfigs: {
      session: { ...DIALOG_FACE_OFF },
      // 启用侧不写这一项：缺省即启用（`PluginDir::enabled` 的约定）。
      ...(localEnabled ? {} : { local: { plugin_enabled: false } }),
    },
  });

  try {
    const r = runCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      message: '你好',
      provider: PROVIDER_ID,
      session: sessionTag,
    });
    assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
    assert(
      r.stdout.includes('收到。'),
      `停用插件不该让会话装载失败，stdout 应有收尾正文（实际: ${JSON.stringify(r.stdout)}）`,
    );

    const reqs = await llm.requests();
    assertEq(reqs.length, 1, `一轮对话应只有一次模型请求，实际 ${reqs.length}`);
    return (reqs[0].body.tools ?? []).map((t) => t.function?.name).filter(Boolean);
  } finally {
    llm.stop();
    cleanupHomedir(hd);
    // 收尾等子进程退干净（Windows 上带活子进程退出会撞 libuv 断言）。
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: 'CLI / mock-llm 子进程退出',
    });
  }
}

export default defineCase(
  'T88 插件接入即能力：停用 local 后其工具从模型工具面消失，启用时出现且装载不红',
  async () => {
    const enabled = await toolNamesFor({ localEnabled: true, sessionTag: 'e2e-t88-on' });
    for (const name of LOCAL_TOOLS) {
      assert(
        enabled.includes(name),
        `启用 local 时工具面应含 ${name}（实际: ${enabled.join(',')}）`,
      );
    }

    const disabled = await toolNamesFor({ localEnabled: false, sessionTag: 'e2e-t88-off' });
    for (const name of LOCAL_TOOLS) {
      assert(
        !disabled.includes(name),
        `停用 local 后工具面不该含 ${name}（实际: ${disabled.join(',')}）`,
      );
    }
    // 负面读数也要有下限：工具面整个空掉会让「不含」白绿，而那意味着别的插件
    // 也被停用了——不是本用例要验的事实。
    assert(disabled.length > 0, '停用 local 只该拿掉它的工具，其余插件的能力应仍在');
  },
);
