// 用例自执行引导：`node e2e/cases/xxx.mjs` 时，用例文件先引入本模块（副作用
// import），defineCase 登记的用例在事件循环空闲后由这里执行并按结果退出。
//
// 仅在自执行标记（E2E_CASE_SELF=1）下生效——runner 或直接运行用例文件时都会
// 设置它；作为普通模块被 import（如未来的聚合工具）不会触发执行。
export async function runRegisteredCase() {
  const cases = globalThis.__E2E_CASES ?? [];
  if (cases.length !== 1) {
    console.error(`[e2e] 自执行模式要求文件恰好定义一个用例（实际 ${cases.length}）`);
    process.exit(2);
  }
  const { name, fn } = cases[0];
  const t0 = Date.now();
  try {
    await fn();
    console.log(`  ✓ ${name} (${Date.now() - t0}ms)`);
    process.exit(0);
  } catch (e) {
    console.error(`  ✗ ${name} (${Date.now() - t0}ms)`);
    console.error(`      ${String(e.message ?? e)}`);
    if (process.env.E2E_DEBUG) console.error(e.stack);
    process.exit(1);
  }
}

if (process.env.E2E_CASE_SELF === '1') {
  setTimeout(() => void runRegisteredCase(), 0);
}
