#!/usr/bin/env node
/**
 * 能力链测试脚本（由 scripts/test_capability_chain.ps1 迁移）
 *
 * 验证完整链路：LLM 调用 → 能力执行 → 能力返回正确结果 → LLM 正确使用结果
 *
 * 用法：
 *   node scripts/test-capability-chain.mjs
 *
 * 环境变量：
 *   CLI_PATH  CLI 可执行文件路径（默认 <repoRoot>/symbio/target/release/cli[.exe]）
 *   WORKDIR   CLI 运行工作目录（默认仓库根）
 *
 * 说明：原 PowerShell 版把 CLI 路径与工作目录硬编码为 c:\Bing\agiwave\... ，
 * 现在改为从仓库根推导（可用上述环境变量覆盖）。输出直接由子进程捕获，
 * 不再落 test_output_*.txt 临时文件。
 */

import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

const CLI_PATH =
  process.env.CLI_PATH ||
  path.join(repoRoot, 'symbio', 'target', 'release', process.platform === 'win32' ? 'cli.exe' : 'cli')
const WORKDIR = process.env.WORKDIR || repoRoot

const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

// ── 测试场景（与原 .ps1 保持一致）──────────────────────────────────────
const testScenarios = [
  {
    name: '1. 知识查询 (agent_query) 测试',
    description: '验证LLM能正确调用查询能力，且查询能力返回正确结果',
    prompt: "请使用 agent_query 查询 ID 为 'sun_rise' 的知识，然后告诉我你找到了什么",
    expected: {
      shouldCallTool: true,
      toolName: 'agent_query',
      shouldFindContent: ['sun_rise', '太阳', '升起', '降雨', '地面变湿'],
      shouldUseResult: true,
    },
  },
  {
    name: '2. 知识存储 (agent_store) 测试',
    description: '验证LLM能正确调用存储能力',
    prompt: "请使用 agent_store 存储一条新知识，ID 为 'test_fact_1'，类型为 'fact'，描述为 '这是一条测试知识'",
    expected: {
      shouldCallTool: true,
      toolName: 'agent_store',
      shouldUseResult: true,
    },
  },
  {
    name: '3. 验证存储结果',
    description: '验证刚才存储的知识能被正确查询到',
    prompt: "请查询一下是否有 ID 为 'test_fact_1' 的知识",
    expected: {
      shouldCallTool: true,
      toolName: 'agent_query',
      shouldFindContent: ['test_fact_1'],
    },
  },
]

// ── 前置检查 ───────────────────────────────────────────────────────────
if (!fs.existsSync(CLI_PATH)) {
  console.error(`[FATAL] 找不到 CLI 可执行文件：${CLI_PATH}`)
  console.error('请先构建（cargo build --release），或用 CLI_PATH 环境变量指定路径。')
  process.exit(1)
}

const results = []

console.log('='.repeat(80))
console.log('  能力链完整测试')
console.log('  验证：LLM调用 → 能力执行 → 能力返回正确结果 → LLM正确使用结果')
console.log('='.repeat(80))
console.log('')

testScenarios.forEach((scenario, idx) => {
  const sessionId = `test_chain_${idx + 1}`

  console.log(`[${scenario.name}]`)
  console.log(`  描述: ${scenario.description}`)
  console.log(`  提示: ${scenario.prompt}`)
  console.log('')

  let result
  try {
    console.log('  执行测试...')

    const proc = spawnSync(
      CLI_PATH,
      ['--agent', 'tester', '--session', sessionId, scenario.prompt],
      { cwd: WORKDIR, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, shell: false },
    )

    const exitCode = proc.status ?? -1
    const fullOutput = `${proc.stdout ?? ''}\n${proc.stderr ?? ''}`
    const outputLines = fullOutput.split(/\r?\n/)

    console.log(`  退出码: ${exitCode}`)
    console.log('')

    const issues = []

    // 1) 能力是否被调用
    const toolCalled = /\[\s*Tool\s*\]\s*Execution\s*started/.test(fullOutput)
    if (toolCalled) {
      console.log('  ✓ 检测到能力调用')
    } else {
      console.log('  ✗ 未检测到能力调用')
      issues.push('未调用能力')
    }

    // 2) 是否调用了正确的能力
    let toolNameMatched = false
    if (scenario.expected.toolName) {
      toolNameMatched = new RegExp(escapeRe(scenario.expected.toolName)).test(fullOutput)
      if (toolNameMatched) {
        console.log(`  ✓ 检测到目标能力: ${scenario.expected.toolName}`)
      } else {
        console.log(`  ✗ 未检测到目标能力: ${scenario.expected.toolName}`)
        issues.push('未调用正确的能力')
      }
    }

    // 3) 是否找到预期内容
    let contentFound = false
    if (scenario.expected.shouldFindContent) {
      const found = scenario.expected.shouldFindContent.filter((kw) =>
        new RegExp(escapeRe(kw)).test(fullOutput),
      )
      contentFound = found.length > 0
      if (contentFound) {
        console.log(`  ✓ 找到预期内容: ${found.join(', ')}`)
      } else {
        console.log('  ✗ 未找到预期内容')
        issues.push('未找到预期内容')
      }
    }

    // 4) LLM 是否使用了能力返回的结果（启发式：末尾 10 行中实质内容行 > 2）
    let resultUsed = false
    if (scenario.expected.shouldUseResult && toolCalled) {
      const answerLines = outputLines
        .filter((l) => /^\s*[^#[\s]/.test(l))
        .slice(-10)
      resultUsed = answerLines.length > 2
      if (resultUsed) console.log('  ✓ LLM基于能力结果给出了回答')
    }

    // 判定：必须调用能力；指定了 toolName 则须匹配；指定了 shouldFindContent 则须找到
    const success =
      toolCalled &&
      (!scenario.expected.toolName || toolNameMatched) &&
      (!scenario.expected.shouldFindContent || contentFound)

    result = {
      scenario: scenario.name,
      toolCalled,
      correctTool: toolNameMatched,
      contentFound,
      resultUsed,
      success,
      issues: issues.join('; '),
      exitCode,
    }

    console.log('')
    console.log(`  结果: ${result.success ? '✓ 通过' : '✗ 失败'}`)
  } catch (e) {
    console.log(`  错误: ${e?.message ?? e}`)
    result = {
      scenario: scenario.name,
      toolCalled: false,
      correctTool: false,
      contentFound: false,
      resultUsed: false,
      success: false,
      issues: `执行错误: ${e?.message ?? e}`,
      exitCode: -1,
    }
  }

  results.push(result)

  console.log('')
  console.log('-'.repeat(80))
  console.log('')
})

// ── 报告 ───────────────────────────────────────────────────────────────
console.log('='.repeat(80))
console.log('  测试报告')
console.log('='.repeat(80))
console.log('')

const total = results.length
const passed = results.filter((r) => r.success).length
const toolCalledRate = results.filter((r) => r.toolCalled).length

console.log('统计:')
console.log(`  总测试数: ${total}`)
console.log(`  通过数: ${passed}`)
console.log(`  能力调用率: ${toolCalledRate}/${total}`)
console.log(`  通过率: ${total === 0 ? '0.0' : ((passed / total) * 100).toFixed(1)}%`)
console.log('')

console.log('详细结果:')
for (const r of results) {
  const mark = r.success ? '✓' : '✗'
  console.log(`  ${mark} ${r.scenario}`)
  console.log(
    `    能力调用: ${r.toolCalled ? '✓' : '✗'}, 正确能力: ${r.correctTool ? '✓' : '✗'}, 内容找到: ${r.contentFound ? '✓' : '✗'}`,
  )
  if (r.issues) console.log(`    问题: ${r.issues}`)
}

console.log('')
console.log('='.repeat(80))
console.log('  测试完成')
console.log('='.repeat(80))
