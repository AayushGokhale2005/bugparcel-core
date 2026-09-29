use anyhow::{Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use parcel_core::ReproductionSpec;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

#[derive(Debug, Serialize)]
pub struct ReplayResult {
    pub observed_exit_code: i32,
    pub matched: bool,
    pub mismatches: Vec<String>,
    pub stdout: String,
    pub stderr: String,
}

pub fn run(spec: &ReproductionSpec, worktree: &Path) -> Result<ReplayResult> {
    run_with_environment(spec, worktree, &[])
}

pub fn run_with_environment(
    spec: &ReproductionSpec,
    worktree: &Path,
    additional_environment: &[(String, String)],
) -> Result<ReplayResult> {
    materialize_fixtures(spec, worktree)?;
    let (program, args) = spec
        .command
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("reproduction command cannot be empty"))?;
    let use_container = spec.environment.container_image.is_some() && docker_daemon_available();
    let output = if use_container {
        let image = spec
            .environment
            .container_image
            .as_ref()
            .expect("checked above");
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
        for (key, value) in additional_environment {
            command.arg("--env").arg(format!("{key}={value}"));
        }
        command.arg(image).arg(program).args(args).output()?
    } else {
        Command::new(program)
            .args(args)
            .current_dir(worktree)
            .envs(&spec.environment.variables)
            .envs(
                additional_environment
                    .iter()
                    .map(|(key, value)| (key, value)),
            )
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

fn docker_daemon_available() -> bool {
    Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn materialize_fixtures(spec: &ReproductionSpec, worktree: &Path) -> Result<()> {
    let Some(state) = &spec.state else {
        return Ok(());
    };
    for fixture in &state.fixtures {
        let relative = Path::new(&fixture.relative_path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            anyhow::bail!("unsafe fixture path: {}", fixture.relative_path);
        }
        let contents = BASE64.decode(&fixture.contents_base64)?;
        if format!("{:x}", Sha256::digest(&contents)) != fixture.sha256 {
            anyhow::bail!("fixture digest mismatch: {}", fixture.relative_path);
        }
        let destination: PathBuf = worktree.join(relative);
        fs::create_dir_all(destination.parent().expect("fixture parent"))?;
        fs::write(destination, contents)?;
    }
    Ok(())
}

pub fn require_match(result: &ReplayResult) -> Result<()> {
    if result.matched {
        Ok(())
    } else {
        bail!("REPRO_ASSERTION_MISMATCH: expected failure signature did not match")
    }
}
