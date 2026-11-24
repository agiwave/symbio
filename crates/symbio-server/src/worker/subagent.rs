use serde_json::Value;

use super::id::validate_session_id;
use super::{InboxRequest, WorkerError, WorkerId};

#[derive(Debug, Clone, PartialEq)]
pub struct StartSubagent {
    parent: WorkerId,
    child_session_id: String,
    request: InboxRequest,
}

impl StartSubagent {
    pub fn new(
        parent: WorkerId,
        child_session_id: impl Into<String>,
        request: InboxRequest,
    ) -> Result<Self, WorkerError> {
        let child_session_id = child_session_id.into();
        validate_session_id(&child_session_id)?;
        Ok(Self {
            parent,
            child_session_id,
            request,
        })
    }

    pub fn parent(&self) -> &WorkerId {
        &self.parent
    }

    pub fn child_session_id(&self) -> &str {
        &self.child_session_id
    }

    pub fn request(&self) -> &InboxRequest {
        &self.request
    }

    pub fn validate(&self, expected_parent: &WorkerId) -> Result<(), WorkerError> {
        self.parent.validate(expected_parent)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubagentRoute {
    parent: WorkerId,
    child_session_id: String,
}

impl SubagentRoute {
    pub fn new(parent: WorkerId, child_session_id: impl Into<String>) -> Result<Self, WorkerError> {
        let child_session_id = child_session_id.into();
        validate_session_id(&child_session_id)?;
        Ok(Self {
            parent,
            child_session_id,
        })
    }

    pub fn parent(&self) -> &WorkerId {
        &self.parent
    }

    pub fn child_session_id(&self) -> &str {
        &self.child_session_id
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubagentFrame {
    route: SubagentRoute,
    child: WorkerId,
    payload: Value,
}

impl SubagentFrame {
    pub fn new(route: SubagentRoute, child: WorkerId, payload: Value) -> Result<Self, WorkerError> {
        if child.session_id() != route.child_session_id {
            return Err(WorkerError::Invalid(
                "子会话 worker 与路由目标不一致".to_string(),
            ));
        }
        Ok(Self {
            route,
            child,
            payload,
        })
    }

    pub fn route(&self) -> &SubagentRoute {
        &self.route
    }

    pub fn child(&self) -> &WorkerId {
        &self.child
    }

    pub fn payload(&self) -> &Value {
        &self.payload
    }

    pub fn validate(
        &self,
        expected_parent: &WorkerId,
        expected_child: &WorkerId,
    ) -> Result<&Value, WorkerError> {
        self.route.parent.validate(expected_parent)?;
        self.child.validate(expected_child)?;
        Ok(&self.payload)
    }
}
