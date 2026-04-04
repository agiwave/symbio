# GitHub Actions 密钥配置助手 (Windows)
# 此脚本帮助生成和配置所有必需的密钥
# 以管理员权限运行此脚本

param(
    [switch]$GenerateTauriKey,
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
    Write-Host "  .\setup-secrets.ps1 [-GenerateTauriKey] [-SkipMacOS]"
    Write-Host ""
    Write-ColorOutput "参数:" "Yellow"
    Write-Host "  -GenerateTauriKey  生成 Tauri 更新签名密钥"
    Write-Host "  -SkipMacOS         跳过 macOS 证书配置指导"
    Write-Host "  -Help              显示此帮助信息"
    Write-Host ""
    Write-ColorOutput "必需密钥:" "Green"
    Write-Host "  ✅ GITHUB_TOKEN (GitHub 自动提供，无需配置)"
    Write-Host ""
    Write-ColorOutput "可选密钥:" "Yellow"
    Write-Host "  - TAURI_SIGNING_PRIVATE_KEY (Tauri 自动更新)"
    Write-Host "  - APPLE_* (macOS 代码签名和公证)"
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

function Generate-TauriSigningKey {
    Write-ColorOutput "`n================================" "Cyan"
    Write-ColorOutput "生成 Tauri 签名密钥" "Cyan"
    Write-ColorOutput "================================" "Cyan"
    Write-Host ""
    
    # 检查 Tauri CLI
    Write-ColorOutput "检查 Tauri CLI..." "Yellow"
    $tauriInstalled = npm list -g @tauri-apps/cli 2>$null
    if ($tauriInstalled -match "@tauri-apps/cli") {
        Write-ColorOutput "✅ Tauri CLI 已安装" "Green"
    } else {
        Write-ColorOutput "⚠️  Tauri CLI 未安装，正在安装..." "Yellow"
        npm install -g @tauri-apps/cli
    }
    
    Write-Host ""
    Write-ColorOutput "开始生成密钥..." "Cyan"
    Write-Host ""
    
    # 生成密钥
    try {
        tauri signer generate
        Write-Host ""
        Write-ColorOutput "✅ 密钥生成成功!" "Green"
        Write-Host ""
        Write-ColorOutput "密钥文件位置:" "Yellow"
        Write-Host "  Windows: $env:USERPROFILE\.tauri\"
        Write-Host ""
        Write-ColorOutput "下一步:" "Yellow"
        Write-Host "  1. 查看私钥: type $env:USERPROFILE\.tauri\.key"
        Write-Host "  2. 复制输出内容"
        Write-Host "  3. 在 GitHub 仓库 Settings → Secrets and variables → Actions 中添加"
        Write-Host "     - Name: TAURI_SIGNING_PRIVATE_KEY"
        Write-Host "     - Secret: 粘贴私钥内容"
        Write-Host ""
    } catch {
        Write-ColorOutput "❌ 密钥生成失败: $_" "Red"
        return $false
    }
    
    return $true
}

function Show-MacOSCertificateGuide {
    Write-ColorOutput "`n================================" "Cyan"
    Write-ColorOutput "macOS 代码签名证书配置指南" "Cyan"
    Write-ColorOutput "================================" "Cyan"
    Write-Host ""
    Write-ColorOutput "如果不配置 macOS 证书，构建的应用会显示'无法验证开发者'" "Yellow"
    Write-Host "用户可以手动绕过，但建议配置证书以获得更好体验。"
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
    Write-Host "  2. Windows 和 Linux 版本会正常生成"
    Write-Host "  3. macOS 版本会生成但有未签名警告"
    Write-Host ""
    Write-ColorOutput "推送新标签触发构建:" "Yellow"
    Write-Host "  git tag v0.2.0"
    Write-Host "  git push origin v0.2.0"
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
    Write-Host "  - TAURI_SIGNING_PRIVATE_KEY (自动更新)"
    Write-Host "  - APPLE_CERTIFICATE 等 (macOS 签名)"
    Write-Host ""
    Write-ColorOutput "📝 详细文档:" "Cyan"
    Write-Host "  请查看: .github/workflows/SECRETS_SETUP.md"
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
$hasAuth = false
if ($hasGitHubCLI) {
    $hasAuth = Check-AuthStatus
}

# 生成 Tauri 密钥
if ($GenerateTauriKey) {
    Generate-TauriSigningKey
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
