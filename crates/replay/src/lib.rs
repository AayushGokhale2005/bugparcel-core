use anyhow::{Result, bail};
use parcel_core::ReproductionSpec;
use serde::Serialize;
use std::{path::Path, process::Command};

#[derive(Debug, Serialize)]
pub struct ReplayResult {
    pub observed_exit_code: i32,
    pub matched: bool,
    pub stdout: String,
    pub stderr: String,
}

pub fn run(spec: &ReproductionSpec, worktree: &Path) -> Result<ReplayResult> {
    let (program, args) = spec
        .command
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("reproduction command cannot be empty"))?;
    let output = Command::new(program)
        .args(args)
        .current_dir(worktree)
        .env("BUGPARCEL_REPLAY", "1")
        .output()?;
    let observed_exit_code = output.status.code().unwrap_or(-1);
    let matched = observed_exit_code == spec.failure_assertion.expected_exit_code;
    Ok(ReplayResult {
        observed_exit_code,
        matched,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

pub fn require_match(result: &ReplayResult) -> Result<()> {
    if result.matched {
        Ok(())
    } else {
        bail!("REPRO_ASSERTION_MISMATCH: expected failure signature did not match")
    }
}
