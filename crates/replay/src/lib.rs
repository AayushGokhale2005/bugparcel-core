use anyhow::{Result, bail};
use parcel_core::ReproductionSpec;
use serde::Serialize;
use std::{path::Path, process::Command};

#[derive(Debug, Serialize)]
pub struct ReplayResult {
    pub observed_exit_code: i32,
    pub matched: bool,
    pub mismatches: Vec<String>,
    pub stdout: String,
    pub stderr: String,
}

pub fn run(spec: &ReproductionSpec, worktree: &Path) -> Result<ReplayResult> {
    let (program, args) = spec
        .command
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("reproduction command cannot be empty"))?;
    let output = if let Some(image) = &spec.environment.container_image {
        let workspace = worktree.canonicalize()?;
        let mut command = Command::new("docker");
        command
            .args([
                "run",
                "--rm",
                "--network",
                "none",
                "--workdir",
                "/workspace",
            ])
            .arg("--volume")
            .arg(format!("{}:/workspace:rw", workspace.display()))
            .args(["--env", "BUGPARCEL_REPLAY=1"]);
        for (key, value) in &spec.environment.variables {
            command.arg("--env").arg(format!("{key}={value}"));
        }
        command.arg(image).arg(program).args(args).output()?
    } else {
        Command::new(program)
            .args(args)
            .current_dir(worktree)
            .envs(&spec.environment.variables)
            .env("BUGPARCEL_REPLAY", "1")
            .output()?
    };
    let observed_exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let combined_output = format!("{stdout}\n{stderr}");
    let mut mismatches = Vec::new();
    if observed_exit_code != spec.failure_assertion.expected_exit_code {
        mismatches.push(format!(
            "exit code mismatch: expected {}, observed {observed_exit_code}",
            spec.failure_assertion.expected_exit_code
        ));
    }
    for expected in &spec.failure_assertion.expected_output_contains {
        if !combined_output.contains(expected) {
            mismatches.push(format!("missing expected output fragment: {expected}"));
        }
    }
    let matched = mismatches.is_empty();
    Ok(ReplayResult {
        observed_exit_code,
        matched,
        mismatches,
        stdout,
        stderr,
    })
}

pub fn require_match(result: &ReplayResult) -> Result<()> {
    if result.matched {
        Ok(())
    } else {
        bail!("REPRO_ASSERTION_MISMATCH: expected failure signature did not match")
    }
}
