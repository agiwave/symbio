//! 中性执行信封：宿主纯值与端口载荷装配（DE-048 D-305 / D-316）。
//! 环只交值；世界语义由执行体自取。

use serde_json::{json, Value};

use symbio_plugin_api::port::PortFactEmit;

use super::gate_inputs::GateInputs;

/// 一笔工具往返的通用信封：字段都是环的通用形式，不含世界语义。
#[derive(Clone, Default)]
pub struct RoundEnv {
    /// 宿主为本轮准备的可写目录（`D-316` 的中性路径值）。
    pub scratch_dir: std::path::PathBuf,
    pub emit: PortFactEmit,
    pub round: u64,
    pub round_id: String,
    pub produced_by: u64,
    pub allowed: bool,
    pub denied_reason: String,
}

/// 调用信封的协议读数：环只搬原始值，判定在执行体（D-312）。
#[derive(Debug, Clone, Default)]
pub struct ProtocolInput {
    pub id_missing: bool,
    pub name_missing: bool,
    pub parse_error: Option<String>,
}

impl ProtocolInput {
    pub(crate) fn is_failure(&self) -> bool {
        self.id_missing || self.name_missing || self.parse_error.is_some()
    }
}

/// 一笔工具债的中性载荷：唯一装配点，不拼世界信封键（`D-316`）。
pub(crate) fn tool_payload(
    name: &str,
    args: Value,
    call_id: &str,
    protocol: &ProtocolInput,
    env: &RoundEnv,
    gate: &GateInputs,
) -> Value {
    json!({
        "name": name,
        "args": args,
        "call_id": call_id,
        "protocol": {
            "id_missing": protocol.id_missing, "name_missing": protocol.name_missing,
            "parse_error": protocol.parse_error,
        },
        "scratch_dir": env.scratch_dir.to_string_lossy(),
        "gate": {
            "authorized": gate.authorized, "spent_ms": gate.spent_ms,
            "requested_ms": gate.requested_ms, "budget_ms": gate.budget_ms,
        },
    })
}
