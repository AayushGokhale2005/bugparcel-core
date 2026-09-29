use anyhow::{Result, bail};
use parcel_core::{ImmutableInput, ReproductionSpec};
use replay::run;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Component, Path},
};

#[derive(Debug)]
pub enum VerificationStatus {
    Verified,
    OriginalFailureRemains,
    AcceptanceFailed { reason: String },
    ImmutableContractChanged { path: String },
}

/// Capture hashes for local files invoked by a replay command. This is small
/// by design: it protects shell/Python test harnesses without trying to freeze
/// an entire repository (which legitimate product fixes must change).
pub fn capture_immutable_inputs(
    repo: &Path,
    spec: &ReproductionSpec,
) -> Result<Vec<ImmutableInput>> {
    let mut inputs = Vec::new();
    for argument in &spec.command {
        let candidate = Path::new(argument);
        if candidate.is_absolute()
            || candidate
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            continue;
        }
        let path = repo.join(candidate);
        if !path.is_file() {
            continue;
        }
        let contents = fs::read(&path)?;
        inputs.push(ImmutableInput {
            relative_path: candidate.display().to_string(),
            sha256: format!("{:x}", Sha256::digest(contents)),
        });
    }
    inputs.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    inputs.dedup_by(|a, b| a.relative_path == b.relative_path);
    Ok(inputs)
}

fn immutable_inputs_unchanged(spec: &ReproductionSpec, worktree: &Path) -> Result<()> {
    for input in &spec.immutable_inputs {
        let path = worktree.join(&input.relative_path);
        let contents = fs::read(&path).map_err(|_| {
            anyhow::anyhow!(
                "IMMUTABLE_ACCEPTANCE_CONTRACT_CHANGED: {} was removed",
                input.relative_path
            )
        })?;
        if format!("{:x}", Sha256::digest(contents)) != input.sha256 {
            bail!(
                "IMMUTABLE_ACCEPTANCE_CONTRACT_CHANGED: {} was modified by the candidate patch",
                input.relative_path
            );
        }
    }
    Ok(())
}

/// A verification run must receive a fresh worktree prepared by the caller.
pub fn verify_patch(spec: &ReproductionSpec, fresh_worktree: &Path) -> Result<VerificationStatus> {
    if let Err(error) = immutable_inputs_unchanged(spec, fresh_worktree) {
        return Ok(VerificationStatus::ImmutableContractChanged {
            path: error.to_string(),
        });
    }
    let result = run(spec, fresh_worktree)?;
    if result.matched {
        return Ok(VerificationStatus::OriginalFailureRemains);
    }
    // A removed failure is necessary but not sufficient. The same immutable
    // replay command is the acceptance harness: a genuine patch must make it
    // finish successfully. This rejects swallowed exceptions and fake return
    // values that merely change the original failure signature.
    if result.observed_exit_code != 0 {
        return Ok(VerificationStatus::AcceptanceFailed {
            reason: format!(
                "acceptance command exited {}; output: {}",
                result.observed_exit_code,
                format!("{}\n{}", result.stdout, result.stderr).trim()
            ),
        });
    }
    Ok(VerificationStatus::Verified)
}
