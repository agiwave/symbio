//! `seed_agents` — 一键创建软件项目开发 7 角色智能体体系
//!
//! ## 用途
//!
//! 批量调用 `agent/create` 路由，创建下列 7 个全局智能体：
//! - `project_manager` 项目经理（任务拆解、调度、汇总）
//! - `architect`        系统架构师（设计、接口定义）
//! - `coder`            开发工程师（写代码）
//! - `reviewer`         代码审查员（Code Review）
//! - `tester`           测试工程师（写/跑测试）
//! - `documenter`       文档工程师（API 文档、用户文档、CHANGELOG）
//! - `devops`           运维/集成（依赖管理、CI、Docker）
//!
//! ## 用法
//!
//! ```bash
//! cargo run --bin seed_agents
//! cargo run --bin seed_agents -- --recreate  # 强制重建（先删后建）
//! ```
//!
//! ## 存储位置
//!
//! `~/.symbio/agents/{agent_id}/` — 全局共享，所有工作区可见
//!
//! ## 失败模式
//!
//! - agent 已存在（且无 `--recreate`）→ 跳过并打印 ⚠️
//! - 路由错误 → 直接 panic，提示用户检查

use serde_json::{json, Value};
use std::env;

use symbio::init::{create_root_plugin, initialize};
use symbio::symbio_core::{
    InvokeRequestExt, Plugin, SimpleRequest, AGENT_CREATE, AGENT_DELETE, AGENT_GET, PATH, WORKDIR,
};

const SEVEN_AGENTS_JSON: &str = include_str!("../plugins/agent/manager/seed_agents_data.json");

fn main() {
    initialize();

    let args: Vec<String> = env::args().collect();
    let recreate = args.iter().any(|a| a == "--recreate");

    let agents: Vec<Value> = serde_json::from_str(SEVEN_AGENTS_JSON)
        .expect("seed_agents_data.json 解析失败 — 请检查文件格式");

    println!("\n🌱 seed_agents — 软件项目开发 7 角色智能体体系");
    println!("═══════════════════════════════════════════════════════");
    println!("  目标: ~/.symbio/agents/");
    println!(
        "  模式: {}\n",
        if recreate {
            "recreate (强制重建)"
        } else {
            "skip-if-exists"
        }
    );

    // 在同步上下文中 spawn 异步运行时（main 不能直接 await）
    let runtime = tokio::runtime::Runtime::new().expect("创建 tokio runtime 失败");
    runtime.block_on(async {
        let root_plugin = create_root_plugin().await;
        let mut ok = 0;
        let mut skipped = 0;
        let mut failed = 0;

        for agent_def in &agents {
            let id = agent_def["id"].as_str().unwrap_or("?").to_string();
            let name = agent_def["name"].as_str().unwrap_or("?").to_string();
            let cus = &agent_def["cognition_units"];

            // ── recreate 模式：先尝试 list，确认存在则跳过删除
            if recreate {
                if let Err(e) = delete_agent(&root_plugin, &id).await {
                    eprintln!("  ⚠️  删 {} 失败（可能不存在）: {}", id, e);
                }
            }

            // ── 检查是否已存在
            match check_exists(&root_plugin, &id).await {
                Ok(true) => {
                    println!("  ⏭️  {} ({}) — 已存在，跳过", id, name);
                    skipped += 1;
                    continue;
                }
                Ok(false) => { /* 不存在，继续创建 */ }
                Err(e) => {
                    eprintln!("  ❌ {} — 检查存在性失败: {}", id, e);
                    failed += 1;
                    continue;
                }
            }

            // ── 调用 agent/create 创建
            let payload = json!({
                "id": id,
                "is_global": true,
                "cognition_units": cus,
            });

            let ctx = std::sync::Arc::new(SimpleRequest::new(None, None));
            ctx.set(PATH, AGENT_CREATE.to_string());
            // workdir = "" 表示走全局目录 ~/.symbio/agents
            ctx.set(WORKDIR, "".to_string());
            ctx.set_payload(payload).expect("set_payload");

            match root_plugin.clone().route(ctx).await {
                Ok(_payload) => {
                    println!("  ✅ {} ({}) — 已创建", id, name);
                    ok += 1;
                }
                Err(e) => {
                    eprintln!("  ❌ {} — 创建失败: {:?}", id, e);
                    failed += 1;
                }
            }
        }

        println!("\n═══════════════════════════════════════════════════════");
        println!("📊 汇总: ✅ {} 创建   ⏭️  {} 跳过   ❌ {} 失败", ok, skipped, failed);
        if failed == 0 {
            println!("🎉 软件项目开发 7 角色智能体体系就绪！");
            println!("\n下一步:");
            println!("  cargo run --bin multi_agent_cli -- list");
            println!("  cargo run --bin multi_agent_cli -- dispatch --agent project_manager --task \"...\"");
            println!("  cargo run --bin multi_agent_cli -- cascade --task \"在 c:\\\\Bing\\\\agiwave\\\\symbio\\\\docs\\\\ 下生成 ARCHITECTURE_REPORT.md\"");
        } else {
            std::process::exit(1);
        }
    });
}

async fn check_exists(
    root_plugin: &std::sync::Arc<dyn Plugin>,
    agent_id: &str,
) -> Result<bool, String> {
    let ctx = std::sync::Arc::new(SimpleRequest::new(None, None));
    ctx.set(PATH, AGENT_GET.to_string());
    ctx.set(WORKDIR, "".to_string());

    // payload = { id: "agent_id" }
    let payload = json!({ "id": agent_id });
    ctx.set_payload(payload)
        .map_err(|e| format!("set_payload: {e}"))?;

    match root_plugin.clone().route(ctx).await {
        Ok(_payload) => Ok(true),
        Err(_) => Ok(false), // NotFound → 不存在
    }
}

async fn delete_agent(
    root_plugin: &std::sync::Arc<dyn Plugin>,
    agent_id: &str,
) -> Result<(), String> {
    // 通过 agent/delete 路由删除（幂等：不存在也返回成功）
    let ctx = std::sync::Arc::new(SimpleRequest::new(None, None));
    ctx.set(PATH, AGENT_DELETE.to_string());
    ctx.set(WORKDIR, "".to_string()); // 全局目录
    let payload = json!({ "id": agent_id });
    ctx.set_payload(payload)
        .map_err(|e| format!("set_payload: {e}"))?;

    match root_plugin.clone().route(ctx).await {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("agent/delete 失败: {e:?}")),
    }
}
