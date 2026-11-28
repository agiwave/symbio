# GitHub Actions 密钥配置助手 (Windows)
# 此脚本帮助生成和配置可选的密钥
# 以管理员权限运行此脚本

param(
    [switch]$SkipMacOS,
    [switch]$Help
)

$ErrorActionPreference = "Stop"

function Write-ColorOutput {
    param(
        [string]$Message,
        [string]$Color = "White"
    )
    Write-Host $Message -ForegroundColor $Color
}

function Show-Help {
    Write-ColorOutput "================================" "Cyan"
    Write-ColorOutput "GitHub Actions 密钥配置助手" "Cyan"
    Write-ColorOutput "================================" "Cyan"
    Write-Host ""
    Write-ColorOutput "用法:" "Yellow"
    Write-Host "  .\setup-secrets.ps1 [-SkipMacOS]"
    Write-Host ""
    Write-ColorOutput "参数:" "Yellow"
    Write-Host "  -SkipMacOS         跳过 macOS 证书配置指导"
    Write-Host "  -Help              显示此帮助信息"
    Write-Host ""
    Write-ColorOutput "必需密钥:" "Green"
    Write-Host "  ✅ GITHUB_TOKEN (GitHub 自动提供，无需配置)"
    Write-Host ""
    Write-ColorOutput "可选密钥:" "Yellow"
    Write-Host "  - APPLE_* (macOS 代码签名，仅 .app 打包时需要)"
    Write-Host ""
}

function Check-GitHubCLI {
    Write-ColorOutput "`n检查 GitHub CLI..." "Cyan"

    try {
        $version = gh --version 2>$null
        if ($version) {
            Write-ColorOutput "✅ GitHub CLI 已安装" "Green"
            return $true
        }
    } catch {
        Write-ColorOutput "❌ 未检测到 GitHub CLI" "Red"
        Write-Host "请先安装: https://cli.github.com/"
        return $false
    }
}

function Check-AuthStatus {
    Write-ColorOutput "`n检查 GitHub 认证状态..." "Cyan"

    $status = gh auth status 2>&1
    if ($LASTEXITCODE -eq 0) {
        Write-ColorOutput "✅ 已认证" "Green"
        return $true
    } else {
        Write-ColorOutput "⚠️  未认证，请先运行:" "Yellow"
        Write-Host "  gh auth login"
        return $false
    }
}

function Show-MacOSCertificateGuide {
    Write-ColorOutput "`n================================" "Cyan"
    Write-ColorOutput "macOS 代码签名证书配置指南（预留）" "Cyan"
    Write-ColorOutput "================================" "Cyan"
    Write-Host ""
    Write-ColorOutput "发布产物是命令行二进制便携包，不受 .app 签名要求约束。" "Yellow"
    Write-Host "本节仅在未来增加 .app / .dmg 打包时才需要。"
    Write-Host ""
    Write-ColorOutput "所需步骤:" "Yellow"
    Write-Host ""

    Write-ColorOutput "1. 加入 Apple Developer Program" "Cyan"
    Write-Host "   - 访问: https://developer.apple.com"
    Write-Host "   - 费用: $99/年"
    Write-Host ""

    Write-ColorOutput "2. 创建 Developer ID Application 证书" "Cyan"
    Write-Host "   - 在 macOS 上使用 Xcode 或 Apple Developer 网站"
    Write-Host "   - 导出为 .p12 格式，设置密码"
    Write-Host ""

    Write-ColorOutput "3. 转换为 Base64" "Cyan"
    Write-Host "   在 macOS 终端运行:"
    Write-Host "   base64 -i certificate.p12 -o certificate.p12.base64"
    Write-Host ""

    Write-ColorOutput "4. 获取签名身份" "Cyan"
    Write-Host "   security find-identity -v -p codesigning"
    Write-Host ""

    Write-ColorOutput "5. 获取团队 ID" "Cyan"
    Write-Host "   访问: https://developer.apple.com/account"
    Write-Host ""

    Write-ColorOutput "6. 生成应用专用密码" "Cyan"
    Write-Host "   访问: https://appleid.apple.com"
    Write-Host "   Sign-In and Security → App-Specific Passwords"
    Write-Host ""

    Write-ColorOutput "7. 在 GitHub 添加以下密钥:" "Yellow"
    Write-Host ""
    Write-Host "   | 名称                       | 值示例                           |"
    Write-Host "   |---------------------------|---------------------------------|"
    Write-Host "   | APPLE_CERTIFICATE          | MII... (Base64 内容)              |"
    Write-Host "   | APPLE_CERTIFICATE_PASSWORD | 导出.p12时设置的密码               |"
    Write-Host "   | APPLE_SIGNING_IDENTITY     | Developer ID Application: ...    |"
    Write-Host "   | APPLE_ID                   | dev@example.com                 |"
    Write-Host "   | APPLE_PASSWORD             | abcd-efgh-ijkl-mnop             |"
    Write-Host "   | APPLE_TEAM_ID              | TEAM123ABC                      |"
    Write-Host ""
}

function Show-QuickStartGuide {
    Write-ColorOutput "`n================================" "Cyan"
    Write-ColorOutput "快速开始指南" "Cyan"
    Write-ColorOutput "================================" "Cyan"
    Write-Host ""
    Write-ColorOutput "最小配置（无需任何密钥）:" "Green"
    Write-Host "  1. 直接推送标签即可触发构建"
    Write-Host "  2. 各平台便携压缩包会正常生成"
    Write-Host "  3. 无需代码签名或自动更新签名"
    Write-Host ""
    Write-ColorOutput "推送新标签触发构建:" "Yellow"
    Write-Host "  git tag v0.1.14"
    Write-Host "  git push origin v0.1.14"
    Write-Host ""
    Write-ColorOutput "查看构建进度:" "Yellow"
    Write-Host "  https://github.com/你的用户名/symbio/actions"
    Write-Host ""
    Write-ColorOutput "下载构建产物:" "Yellow"
    Write-Host "  构建完成后，在 GitHub Releases 页面下载"
    Write-Host ""
}

function Show-SetupSummary {
    Write-ColorOutput "`n================================" "Cyan"
    Write-ColorOutput "配置总结" "Cyan"
    Write-ColorOutput "================================" "Cyan"
    Write-Host ""
    Write-ColorOutput "✅ 必需密钥:" "Green"
    Write-Host "  - GITHUB_TOKEN (自动提供)"
    Write-Host ""
    Write-ColorOutput "⚙️  可选密钥:" "Yellow"
    Write-Host "  - APPLE_CERTIFICATE 等 (macOS 签名，仅 .app 打包时需要)"
    Write-Host ""
    Write-ColorOutput "📝 详细文档:" "Cyan"
    Write-Host "  请查看: .github/workflows/002-SECRETS_SETUP.md"
    Write-Host ""
    Write-ColorOutput "🚀 现在可以开始发布了!" "Green"
    Write-Host ""
}

# 主逻辑
if ($Help) {
    Show-Help
    exit 0
}

Write-ColorOutput "`n================================" "Cyan"
Write-ColorOutput "GitHub Actions 密钥配置助手" "Cyan"
Write-ColorOutput "================================" "Cyan"
Write-Host ""

# 检查 GitHub CLI
$hasGitHubCLI = Check-GitHubCLI
# 必须用 $false 而不是 false：裸 `false` 在 PowerShell 中是常量表达式，
# 在某些上下文（如 as-value-in-arg）会被当成 string "False"。
# 全脚本统一用 $false / $true
$hasAuth = $false
if ($hasGitHubCLI) {
    $hasAuth = Check-AuthStatus
}

# 显示 macOS 证书指南
if (-not $SkipMacOS) {
    Show-MacOSCertificateGuide
}

# 显示快速开始
Show-QuickStartGuide

# 显示总结
Show-SetupSummary

Write-ColorOutput "按任意键退出..." "Gray"
$null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
