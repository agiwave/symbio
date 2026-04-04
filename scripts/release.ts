/**
 * 自动发布脚本
 * 
 * 功能：
 * 1. 升级版本号（major/minor/patch）
 * 2. 更新所有版本相关文件（package.json, Cargo.toml, tauri.conf.json）
 * 3. 提交 git 变更
 * 4. 推送代码到远程仓库
 * 5. 创建并推送 release tag
 * 
 * 使用方法：
 *   npx tsx scripts/release.ts [major|minor|patch]  # 默认: patch
 *   npx tsx scripts/release.ts                       # 等同于 patch
 *   npx tsx scripts/release.ts minor                 # 升级次版本号
 *   npx tsx scripts/release.ts major                 # 升级主版本号
 */

import { readFileSync, writeFileSync, existsSync } from 'fs'
import { join, resolve } from 'path'
import { execSync } from 'child_process'
import { createInterface } from 'readline'

// 项目根目录
const PROJECT_ROOT = resolve(__dirname, '..')

// 版本文件路径
const VERSION_FILES = {
  packageJson: join(PROJECT_ROOT, 'package.json'),
  cargoToml: join(PROJECT_ROOT, 'src-tauri', 'Cargo.toml'),
  tauriConf: join(PROJECT_ROOT, 'src-tauri', 'tauri.conf.json'),
}

// 颜色输出
const colors = {
  reset: '\x1b[0m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  red: '\x1b[31m',
  cyan: '\x1b[36m',
  bold: '\x1b[1m',
}

function log(message: string, color: string = colors.reset) {
  console.log(`${color}${message}${colors.reset}`)
}

function error(message: string) {
  console.error(`${colors.red}❌ ${message}${colors.reset}`)
  process.exit(1)
}

function success(message: string) {
  log(`✅ ${message}`, colors.green)
}

function info(message: string) {
  log(`ℹ️  ${message}`, colors.blue)
}

function warn(message: string) {
  log(`⚠️  ${message}`, colors.yellow)
}

function exec(command: string, silent = false) {
  try {
    const result = execSync(command, { cwd: PROJECT_ROOT, encoding: 'utf-8' })
    if (!silent) {
      log(result.trim(), colors.cyan)
    }
    return result.trim()
  } catch (err: any) {
    error(`命令执行失败: ${command}\n${err.message}`)
  }
}

// 解析版本号
interface Version {
  major: number
  minor: number
  patch: number
}

function parseVersion(versionStr: string): Version {
  const match = versionStr.match(/^(\d+)\.(\d+)\.(\d+)/)
  if (!match) {
    error(`无效的版本号: ${versionStr}`)
  }
  return {
    major: parseInt(match[1], 10),
    minor: parseInt(match[2], 10),
    patch: parseInt(match[3], 10),
  }
}

function formatVersion(version: Version): string {
  return `${version.major}.${version.minor}.${version.patch}`
}

function bumpVersion(version: Version, type: 'major' | 'minor' | 'patch'): Version {
  switch (type) {
    case 'major':
      return { major: version.major + 1, minor: 0, patch: 0 }
    case 'minor':
      return { major: version.major, minor: version.minor + 1, patch: 0 }
    case 'patch':
      return { major: version.major, minor: version.minor, patch: version.patch + 1 }
  }
}

// 读取当前版本号
function getCurrentVersion(): Version {
  if (!existsSync(VERSION_FILES.packageJson)) {
    error('找不到 package.json 文件')
  }

  const pkg = JSON.parse(readFileSync(VERSION_FILES.packageJson, 'utf-8'))
  if (!pkg.version) {
    error('package.json 中没有找到 version 字段')
  }

  return parseVersion(pkg.version)
}

// 更新 package.json
function updatePackageJson(version: string) {
  const filePath = VERSION_FILES.packageJson
  const content = readFileSync(filePath, 'utf-8')
  const pkg = JSON.parse(content)

  pkg.version = version

  writeFileSync(filePath, JSON.stringify(pkg, null, 2) + '\n', 'utf-8')
  success(`已更新 package.json: ${version}`)
}

// 更新 Cargo.toml
function updateCargoToml(version: string) {
  const filePath = VERSION_FILES.cargoToml
  let content = readFileSync(filePath, 'utf-8')

  // 替换 version = "x.x.x"
  content = content.replace(
    /^version = ".*"$/m,
    `version = "${version}"`
  )

  writeFileSync(filePath, content, 'utf-8')
  success(`已更新 Cargo.toml: ${version}`)
}

// 更新 tauri.conf.json
function updateTauriConf(version: string) {
  const filePath = VERSION_FILES.tauriConf
  const content = readFileSync(filePath, 'utf-8')
  const config = JSON.parse(content)

  config.version = version

  writeFileSync(filePath, JSON.stringify(config, null, 2) + '\n', 'utf-8')
  success(`已更新 tauri.conf.json: ${version}`)
}

// 检查 git 状态
function checkGitStatus() {
  info('检查 git 状态...')
  const status = exec('git status --porcelain', true)

  if (status) {
    warn('工作区有未提交的变更，建议先提交或暂存')
  }

  // 检查是否在 main 分支
  const branch = exec('git branch --show-current', true)
  if (branch !== 'main') {
    warn(`当前分支: ${branch}，建议在 main 分支上发布`)
  }

  // 检查是否有远程 origin
  const remote = exec('git remote -v', true)
  if (!remote.includes('origin')) {
    error('未找到远程仓库 origin')
  }
}

// 主函数
async function main() {
  console.log('')
  log('═══════════════════════════════════════', colors.cyan)
  log('🚀 Symbio 自动发布脚本', colors.cyan)
  log('═══════════════════════════════════════', colors.cyan)
  console.log('')

  // 解析参数
  const args = process.argv.slice(2)
  const bumpType = (args[0] as 'major' | 'minor' | 'patch') || 'patch'

  if (!['major', 'minor', 'patch'].includes(bumpType)) {
    error(`无效的升级类型: ${bumpType}，可选: major, minor, patch`)
  }

  // 获取当前版本
  const currentVersion = getCurrentVersion()
  const currentVersionStr = formatVersion(currentVersion)
  log(`📦 当前版本: ${colors.bold}${currentVersionStr}`, colors.blue)

  // 计算新版本
  const newVersion = bumpVersion(currentVersion, bumpType)
  const newVersionStr = formatVersion(newVersion)
  log(`🆕 新版本: ${colors.bold}${newVersionStr} (${bumpType})`, colors.green)
  console.log('')

  // 确认操作
  const rl = createInterface({
    input: process.stdin,
    output: process.stdout,
  })

  const answer = await new Promise<string>((resolve) => {
    rl.question(`${colors.yellow}是否继续发布 v${newVersionStr}? (y/N): ${colors.reset}`, resolve)
  })
  rl.close()

  if (answer.toLowerCase() !== 'y' && answer.toLowerCase() !== 'yes') {
    log('已取消发布', colors.yellow)
    process.exit(0)
  }

  console.log('')
  log('📝 开始发布流程...', colors.cyan)
  console.log('')

  // 步骤 1: 更新版本文件
  log('步骤 1/5: 更新版本文件', colors.bold)
  updatePackageJson(newVersionStr)
  updateCargoToml(newVersionStr)
  updateTauriConf(newVersionStr)
  console.log('')

  // 步骤 2: Git 提交
  log('步骤 2/5: 提交版本变更', colors.bold)
  exec(`git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json`)
  exec(`git commit -m "chore: bump version to v${newVersionStr}"`)
  success(`已提交版本变更`)
  console.log('')

  // 步骤 3: 推送代码
  log('步骤 3/5: 推送代码到远程仓库', colors.bold)
  exec(`git push origin HEAD`)
  success(`代码已推送到远程仓库`)
  console.log('')

  // 步骤 4: 创建并推送 tag
  log('步骤 4/5: 创建 Release Tag', colors.bold)
  exec(`git tag v${newVersionStr}`)
  exec(`git push origin v${newVersionStr}`)
  success(`已创建并推送 tag: v${newVersionStr}`)
  console.log('')

  // 步骤 5: 触发 GitHub Actions
  log('步骤 5/5: GitHub Actions 将自动构建', colors.bold)
  info(`访问: https://github.com/agiwave/symbio/actions`)
  info(`构建完成后，在 Releases 页面查看并正式发布`)
  console.log('')

  // 完成
  log('═══════════════════════════════════════', colors.green)
  success(`🎉 发布成功！`)
  log(`📦 版本: v${newVersionStr}`, colors.green)
  log(`🔗 Actions: https://github.com/agiwave/symbio/actions`, colors.cyan)
  log(`🔗 Releases: https://github.com/agiwave/symbio/releases`, colors.cyan)
  log('═══════════════════════════════════════', colors.green)
  console.log('')
}

main().catch((err) => {
  error(`发布失败: ${err.message}`)
  console.error(err)
  process.exit(1)
})
