//! 本机主体清单：`(主体, 能力名, 可见域)` —— **由 `scripts/facts/authz-rows.mjs` 生成，请勿手改**。
//!
//! 事实源：各插件自己目录里的 `PLUGIN.yml` 的 `principal_*` 行（加一个主体 = 改
//! **自己目录**里的清单；手改本文件会被 `gen-facts --check` 判红）。
//!
//! # 这张表是什么
//!
//! 「谁持有什么」与「能不能」是同一张表的两半（模块头的三方边界）：表是**部署事实**
//! ——本机有哪几个主体、各持有什么能力、内容给谁看。能力名的闭集与拼写由
//! `PermissionMatrix::from_names` 校验（认不出即拒绝构造），本文件只负责把行摆出来。
//!
//! # 读法
//!
//! 每行三格：主体（可路由地址 `<namespace>/<name>`）、能力名列表、可见域。
//! **窄行是唯一入口**（D-099）：未登记的主体在表里没有行 ⇒ 每个谓词为假，
//! 「忘配窄行」的后果是一条**会被看见的拒绝**，不是一次静默超权。
//!
//! 名字与理由住在 `authz/mod.rs` 的 `PRINCIPAL_*` 常量上（那里写着每行为什么这么窄）；
//! 生成器双向核对两者：常量集合 ≠ 清单集合即抛错。

use super::{PRINCIPAL_ARBITER, PRINCIPAL_AUTONOMOUS, PRINCIPAL_GATEKEEPER, PRINCIPAL_MAIN, PRINCIPAL_MARKET, PRINCIPAL_PEER, PRINCIPAL_PLANNER, PRINCIPAL_REFLEX, PRINCIPAL_SCHEDULER, PRINCIPAL_TOOL_CONTENT_SEARCH, PRINCIPAL_TOOL_VDFS_WRITE, PRINCIPAL_TOOL_WEB_FETCH, PRINCIPAL_VERIFIER, PRINCIPAL_WORKER};
use super::VisScope;

/// 本机主体清单（生成物；理由住在 `authz/mod.rs` 的常量文档上）。
#[rustfmt::skip]
pub(crate) const ROWS: &[(&str, &[&str], VisScope)] = &[
    // ============ local ============
    (PRINCIPAL_TOOL_CONTENT_SEARCH, &["produce.artifact"], VisScope::ThreadPrivate),
    // ============ loop ============
    (PRINCIPAL_SCHEDULER, &["assign.work"], VisScope::ThreadPrivate),
    // ============ session ============
    (PRINCIPAL_MAIN, &["judge.intent", "reply", "define.work", "produce.artifact", "assign.work"], VisScope::ThreadPrivate),
    (PRINCIPAL_AUTONOMOUS, &["define.work"], VisScope::ThreadPrivate),
    (PRINCIPAL_REFLEX, &["judge.intent"], VisScope::ThreadPrivate),
    (PRINCIPAL_PEER, &["produce.artifact"], VisScope::ThreadPrivate),
    (PRINCIPAL_MARKET, &["assign.work"], VisScope::Public),
    (PRINCIPAL_ARBITER, &["assert.verification"], VisScope::Public),
    (PRINCIPAL_GATEKEEPER, &["judge.intent"], VisScope::ThreadPrivate),
    (PRINCIPAL_WORKER, &["produce.artifact"], VisScope::ThreadPrivate),
    (PRINCIPAL_PLANNER, &["define.work"], VisScope::ThreadPrivate),
    (PRINCIPAL_VERIFIER, &["assert.verification"], VisScope::ThreadPrivate),
    // ============ vdfs ============
    (PRINCIPAL_TOOL_VDFS_WRITE, &["produce.artifact"], VisScope::ThreadPrivate),
    // ============ web ============
    (PRINCIPAL_TOOL_WEB_FETCH, &["produce.artifact"], VisScope::ThreadPrivate),
];
