use anyhow::Result;
use parcel_core::ReproductionSpec;
use replay::run;
use std::path::Path;

pub enum VerificationStatus {
    Verified,
    OriginalFailureRemains,
}

/// A verification run must receive a fresh worktree prepared by the caller.
pub fn verify_patch(spec: &ReproductionSpec, fresh_worktree: &Path) -> Result<VerificationStatus> {
    let result = run(spec, fresh_worktree)?;
    Ok(if result.matched {
        VerificationStatus::OriginalFailureRemains
    } else {
        VerificationStatus::Verified
    })
}
