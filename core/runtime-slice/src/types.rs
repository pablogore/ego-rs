use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuntimeSliceId(String);

impl RuntimeSliceId {
    pub fn new(value: impl Into<String>) -> Result<Self, RuntimeSliceError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(RuntimeSliceError::AmbiguousInput("runtime slice id is empty"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterministicInput {
    pub key: String,
    pub value: String,
}

impl DeterministicInput {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Result<Self, RuntimeSliceError> {
        let key = key.into();
        if key.trim().is_empty() {
            return Err(RuntimeSliceError::AmbiguousInput("deterministic input key is empty"));
        }
        Ok(Self {
            key,
            value: value.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionContext {
    pub slice_id: RuntimeSliceId,
    pub inputs: Vec<DeterministicInput>,
}

impl ExecutionContext {
    pub fn new(
        slice_id: RuntimeSliceId,
        inputs: Vec<DeterministicInput>,
    ) -> Result<Self, RuntimeSliceError> {
        if inputs.is_empty() {
            return Err(RuntimeSliceError::AmbiguousInput(
                "execution context has no governed inputs",
            ));
        }
        Ok(Self { slice_id, inputs })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionOutcome {
    pub slice_id: RuntimeSliceId,
    pub observable_semantics: Vec<String>,
}

impl ExecutionOutcome {
    pub fn new(
        slice_id: RuntimeSliceId,
        observable_semantics: Vec<String>,
    ) -> Result<Self, RuntimeSliceError> {
        if observable_semantics.is_empty() {
            return Err(RuntimeSliceError::AmbiguousOutcome(
                "execution outcome has no observable semantics",
            ));
        }
        Ok(Self {
            slice_id,
            observable_semantics,
        })
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RuntimeSliceError {
    #[error("ambiguous input: {0}")]
    AmbiguousInput(&'static str),
    #[error("ambiguous outcome: {0}")]
    AmbiguousOutcome(&'static str),
}
