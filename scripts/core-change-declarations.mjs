/**
 * 后继提交补签（）：本地未推送的一批提交里，后一笔可以为前一笔缺的
 * `Core-Change:` / 补件决策号显式签名，不必 amend / rebase 改写历史。
 *
 * 签名写在**后继提交**的正文里，一行一项：
 *
 *     Core-Change-For: <前一笔提交号> <判据码> <为什么这笔必须改机制面>
 *
 * 这不是放宽判据：目标必须在本批范围内、严格早于签名提交、规则码形状合法、理由非空，
 * 且同一目标只能签一次。签名只替目标补「缺署名」这一类错误；
 * 其它判据仍照原样判。目标一旦被重写（sha 变了）或移出本批，签名即失效。
 */
// @gate lib

const DECLARATION_LINE = /^\s*Core-Change-For[:：]\s*([0-9a-f]{7,40})\s+([A-Z]{1,4}-\d{3})\s+(.+)$/i

/**
 * 从本批提交正文里收集后继补签，并校验其形状。
 *
 * @param commits [{ sha, message }]，按提交顺序（旧 → 新）
 */
export function collectDeclarations(commits, repoRoot = process.cwd()) {
  const declarations = new Map()
  const errors = []
  const positions = new Map(commits.map((c, i) => [c.sha.slice(0, 8), i]))

  for (const [index, commit] of commits.entries()) {
    const by = commit.sha.slice(0, 8)
    for (const line of String(commit.message ?? '').split(/\r?\n/)) {
      const match = line.match(DECLARATION_LINE)
      if (!match) continue
      const [, rawSha, rule, reason] = match
      const target = rawSha.slice(0, 8)
      const where = `${by} 的 Core-Change-For`

      if (reason.trim().length < 8) {
        errors.push(`${where}：当前理由太短——补签要写清这笔为什么必须改机制面`)
      }
      if (target === by) {
        errors.push(`${where}：不能给自己补签——缺署名应由后继提交声明`)
      }
      if (!positions.has(target)) {
        errors.push(`${where}：目标 ${target} 不在本批提交里（重写或已推送的提交不能用补签）`)
      } else if (positions.get(target) >= index) {
        errors.push(`${where}：目标 ${target} 必须严格早于签名提交`)
      }
      if (declarations.has(target)) {
        errors.push(`${where}：目标 ${target} 已被 ${declarations.get(target).by} 补签——一笔只能签一次`)
      } else {
        declarations.set(target, { target, rule, reason: reason.trim(), by })
      }
    }
  }
  return { declarations, errors }
}
