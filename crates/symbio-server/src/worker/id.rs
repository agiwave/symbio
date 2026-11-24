use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::WorkerError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Epoch(u64);

impl Epoch {
    pub fn new(value: u64) -> Option<Self> {
        (value != 0).then_some(Self(value))
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Epoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:x}", self.0)
    }
}

impl Serialize for Epoch {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.0)
    }
}

impl<'de> Deserialize<'de> for Epoch {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = u64::deserialize(deserializer)?;
        Self::new(value).ok_or_else(|| serde::de::Error::custom("epoch 必须大于 0"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BuildId(String);

impl BuildId {
    pub fn parse(value: impl Into<String>) -> Result<Self, WorkerError> {
        let value = value.into();
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(WorkerError::Invalid(
                "build_id 必须是 64 位小写十六进制".to_string(),
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BuildId {
    type Error = WorkerError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<BuildId> for String {
    fn from(value: BuildId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RequestId(String);

impl RequestId {
    pub fn parse(value: impl Into<String>) -> Result<Self, WorkerError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 96
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(WorkerError::Invalid(
                "request_id 只能含 ASCII 字母、数字、- 或 _，且长度为 1..=96".to_string(),
            ));
        }
        Ok(Self(value))
    }

    pub fn generated(prefix: char, parts: &[u64]) -> Self {
        let mut value = prefix.to_string();
        for part in parts {
            value.push_str(&format!("{part:x}"));
        }
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for RequestId {
    type Error = WorkerError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<RequestId> for String {
    fn from(value: RequestId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkerId {
    session_id: String,
    epoch: Epoch,
    build_id: BuildId,
}

impl Serialize for WorkerId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Record<'a> {
            session_id: &'a str,
            epoch: Epoch,
            build_id: &'a BuildId,
        }
        Record {
            session_id: &self.session_id,
            epoch: self.epoch,
            build_id: &self.build_id,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for WorkerId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Record {
            session_id: String,
            epoch: Epoch,
            build_id: BuildId,
        }
        let record = Record::deserialize(deserializer)?;
        Self::new(record.session_id, record.epoch, record.build_id)
            .map_err(serde::de::Error::custom)
    }
}

impl WorkerId {
    pub fn new(
        session_id: impl Into<String>,
        epoch: Epoch,
        build_id: BuildId,
    ) -> Result<Self, WorkerError> {
        let session_id = session_id.into();
        validate_session_id(&session_id)?;
        Ok(Self {
            session_id,
            epoch,
            build_id,
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn build_id(&self) -> &BuildId {
        &self.build_id
    }

    pub(super) fn key(&self) -> String {
        format!("e{}-{}", self.epoch, self.build_id.as_str())
    }

    pub(super) fn validate(&self, expected: &Self) -> Result<(), WorkerError> {
        if self == expected {
            Ok(())
        } else {
            Err(WorkerError::StaleWorker {
                expected: expected.clone(),
                actual: self.clone(),
            })
        }
    }
}

impl fmt::Display for WorkerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}@e{}:{}",
            self.session_id,
            self.epoch,
            self.build_id.as_str()
        )
    }
}

pub(super) fn validate_session_id(session_id: &str) -> Result<(), WorkerError> {
    if session_id.is_empty()
        || session_id.len() > 128
        || matches!(session_id, "." | "..")
        || session_id
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\' | ':'))
    {
        return Err(WorkerError::Invalid(
            "session_id 不能为空、含路径分隔符/冒号/控制字符，且长度不得超过 128".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fenced<T> {
    pub worker: WorkerId,
    pub body: T,
}

impl<T> Fenced<T> {
    pub fn new(worker: WorkerId, body: T) -> Self {
        Self { worker, body }
    }

    pub fn validate(&self, expected: &WorkerId) -> Result<&T, WorkerError> {
        self.worker.validate(expected)?;
        Ok(&self.body)
    }
}
