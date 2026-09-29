use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ulid::Ulid;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ParcelId(String);

impl ParcelId {
    pub fn new() -> Self {
        Self(format!("bp_{}", Ulid::new()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ParcelId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParcelStatus {
    Draft,
    Captured,
    Reducing,
    Reproducing,
    ReproFailed,
    Reproducible,
    Shared,
    FixProposed,
    Verifying,
    Verified,
    VerifyFailed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StatusEvent {
    pub at: DateTime<Utc>,
    pub from: ParcelStatus,
    pub to: ParcelStatus,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GitState {
    pub repository_path: String,
    pub commit_sha: String,
    pub branch_hint: Option<String>,
    pub staged_patch: Option<String>,
    pub working_tree_patch: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FailureAssertion {
    pub expected_exit_code: i32,
    #[serde(default)]
    pub expected_output_contains: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReproductionSpec {
    pub command: Vec<String>,
    pub failure_assertion: FailureAssertion,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub schema_version: String,
    pub parcel_id: ParcelId,
    pub created_at: DateTime<Utc>,
    pub status: ParcelStatus,
    pub source: GitState,
    pub reproduction: ReproductionSpec,
    pub status_events: Vec<StatusEvent>,
}

#[derive(Debug, Error)]
pub enum TransitionError {
    #[error("invalid transition from {from:?} to {to:?}")]
    Invalid {
        from: ParcelStatus,
        to: ParcelStatus,
    },
}

impl Manifest {
    pub fn new(source: GitState, reproduction: ReproductionSpec) -> Self {
        Self {
            schema_version: "0.1.0".into(),
            parcel_id: ParcelId::new(),
            created_at: Utc::now(),
            status: ParcelStatus::Draft,
            source,
            reproduction,
            status_events: vec![],
        }
    }

    pub fn transition(
        &mut self,
        to: ParcelStatus,
        reason: Option<String>,
    ) -> Result<(), TransitionError> {
        let valid = matches!(
            (self.status, to),
            (ParcelStatus::Draft, ParcelStatus::Captured)
                | (ParcelStatus::Captured, ParcelStatus::Reducing)
                | (
                    ParcelStatus::Captured | ParcelStatus::Reducing,
                    ParcelStatus::Reproducing
                )
                | (
                    ParcelStatus::Reproducing,
                    ParcelStatus::ReproFailed | ParcelStatus::Reproducible
                )
                | (
                    ParcelStatus::Reproducible,
                    ParcelStatus::Shared | ParcelStatus::FixProposed
                )
                | (ParcelStatus::FixProposed, ParcelStatus::Verifying)
                | (
                    ParcelStatus::Verifying,
                    ParcelStatus::Verified | ParcelStatus::VerifyFailed
                )
        );
        if !valid {
            return Err(TransitionError::Invalid {
                from: self.status,
                to,
            });
        }
        self.status_events.push(StatusEvent {
            at: Utc::now(),
            from: self.status,
            to,
            reason,
        });
        self.status = to;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transitions_are_append_only() {
        let mut manifest = Manifest::new(
            GitState {
                repository_path: "/tmp/repo".into(),
                commit_sha: "abc".into(),
                branch_hint: None,
                staged_patch: None,
                working_tree_patch: None,
            },
            ReproductionSpec {
                command: vec!["false".into()],
                failure_assertion: FailureAssertion {
                    expected_exit_code: 1,
                    expected_output_contains: vec![],
                    context: None,
                },
            },
        );
        manifest.transition(ParcelStatus::Captured, None).unwrap();
        manifest
            .transition(ParcelStatus::Reproducing, None)
            .unwrap();
        manifest
            .transition(ParcelStatus::Reproducible, None)
            .unwrap();
        assert_eq!(manifest.status_events.len(), 3);
    }
}
