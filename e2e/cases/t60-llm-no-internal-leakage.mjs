import './_selfrun.mjs';
// T60 无内部泄漏契约（LC-006）：user 消息只承载用户原话，prompt 不掺环境细节。
//
// ## 本用例钉的是什么
//
// 泄漏是另一类「安静失效」：把 homedir / 端口 / 绝对路径拼进提示词，或者把
// 系统前缀包进用户发言——模型照常回答，用户照常看到回复，但「在哪台机器上跑」
// 悄悄改变了模型看到的内容（可复现性没了），「用户说了什么」也不再是字面事实
// （mock 场景匹配、轮次窗口、记忆编码全按它读）。
//
// 两条判据：
// ① user 消息正文 == 用户原话（时间上下文走 `prompt` 字段是既定设计，
//    `message_build.rs::temporal_context`；正文必须保持原话）；
// ② system 消息不含 homedir / workdir / mock 端口 / 回环地址（记忆地址用
//    相对路径 `.vdfsv2/...` 是既定设计，不在判据范围）。
//
// ## 为什么带「判据自检」
//
// 「absence 类」断言最危险的失效是**空转**（写错了字段、判了个永远为真的条件）——
// 它不红，于是看起来一直绿。按本仓验证纪律（DE-016 §5.1：每条断言
// 配一个反向用例），本用例在末尾用合成请求做两步自检：坏输入必须抛、好输入
// 必须过。两步任一不成立，说明判据本身坏了——那比被测系统坏了更严重。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  runCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';
import {
  assertUserTextPure,
  assertNoEnvLeak,
  systemTextOf,
  messagesOf,
} from '../llm-contract.mjs';

const U1 = '第一句：今天天气如何';
const U2 = '第二句：那明天呢';

/** 合成一条带 system 的请求（判据自检用；形状与真实请求同构） */
function fakeReq(systemText) {
  return { body: { messages: [{ role: 'system', content: systemText }, { role: 'user', content: U1 }] } };
}

export default defineCase(
  'T60 无内部泄漏：user 消息纯度 + prompt 无环境细节（LC-006，含判据自检反向用例）',
  async () => {
    const llm = await new MockLlm([
      { id: 't1', match: U1, content: '晴天。' },
      { id: 't2', match: U2, content: '多云。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: { session: { ...DIALOG_FACE_OFF } },
    });

    /** 断言必须抛（反向用例 helper） */
    const mustThrow = (fn, what) => {
      try {
        fn();
      } catch {
        return; // 期望的抛
      }
      throw new Error(`判据自检失败：${what} 应当抛错但没有抛（判据空转）`);
    };

    try {
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        session: 'e2e-t60',
        stdinText: `${U1}\n${U2}\n`,
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 600)}）`);

      const reqs = await llm.requests();
      assertEq(reqs.length, 2, '两轮各一次请求（对话面已关）');

      // ── ① user 消息纯度：正文恰为原话，无前缀无包装 ─────────────────────
      assertUserTextPure(reqs[0], U1, 'T60① 第一轮');
      assertUserTextPure(reqs[1], U2, 'T60① 第二轮');
      // 原话不得被包进别的位置（system / assistant 都不该有用户原话的副本）
      const sysHasU1 = systemTextOf(reqs[0]).includes(U1);
      assert(!sysHasU1, 'system 提示词不得夹带用户原话（那是把用户发言当指令前缀的形态）');

      // ── ② prompt 无环境细节 ─────────────────────────────────────────────
      for (const [i, q] of reqs.entries()) {
        assertNoEnvLeak(q, { homedir: hd.homedir, workdir: hd.workdir, port: llm.port }, `T60② 第 ${i + 1} 轮`);
      }
      // 记忆地址用相对路径（既定设计）——顺带钉住，防止「修泄漏」把地址也删了
      assert(
        systemTextOf(reqs[0]).includes('.vdfsv2/'),
        '记忆段地址应为相对路径 .vdfsv2/...（无环境泄漏与地址可读并存）',
      );

      // ── ③ 判据自检（反向用例：坏输入必须抛）─────────────────────────────
      mustThrow(
        () => assertUserTextPure(fakeReq('x'), '一句不存在的话', '自检'),
        'assertUserTextPure 对不存在的原话',
      );
      mustThrow(
        () =>
          assertNoEnvLeak(fakeReq(`临时目录 ${hd.homedir} 泄漏`), { homedir: hd.homedir }, '自检'),
        'assertNoEnvLeak 对含 homedir 的 system',
      );
      mustThrow(
        () => assertNoEnvLeak(fakeReq(`连端口 127.0.0.1:${llm.port}`), { port: llm.port }, '自检'),
        'assertNoEnvLeak 对含 mock 端口的 system',
      );
      // 好输入必须过（否则抛错那条可能是「永远抛」的另一极）
      assertUserTextPure(fakeReq('x'), U1, '自检：好输入');
      assertNoEnvLeak(fakeReq('【工作区记忆】（地址：.vdfsv2/memory/WORKSPACE.md）'), { homedir: hd.homedir }, '自检：好输入');
      // 形状自检：fakeReq 真的是「一条 system 在头」的真实形状（自检不能建在
      // 一个失真的夹具上——那两类错误互相掩盖）
      assertEq(messagesOf(fakeReq('x'))[0].role, 'system', '自检夹具形状');
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
