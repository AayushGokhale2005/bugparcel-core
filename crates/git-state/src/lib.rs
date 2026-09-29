use anyhow::{Context, Result, bail};
use parcel_core::{GitState, SubmoduleState};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn git_output(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .context("run git")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    Ok(git_output(repo, args)?.trim().to_owned())
}

/// A stable, user-actionable error contract for a parcel whose commit is not
/// present in the recipient clone. Callers intentionally surface this text
/// unchanged through both the CLI and MCP transports.
fn missing_object(commit: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "REPRO_MISSING_GIT_OBJECT: captured commit {commit} is not available locally. Run `git fetch --all` (or `git fetch <remote> {commit}`) and retry."
    )
}

fn ensure_object_exists(repo: &Path, commit: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["cat-file", "-e", &format!("{commit}^{{commit}}")])
        .current_dir(repo)
        .output()
        .context("check captured Git object")?;
    if output.status.success() {
        Ok(())
    } else {
        Err(missing_object(commit))
    }
}

fn capture_submodules(repo: &Path) -> Result<Vec<SubmoduleState>> {
    let output = git_output(repo, &["submodule", "status", "--recursive"])?;
    let mut submodules = Vec::new();
    for line in output.lines() {
        // `git submodule status` prefixes SHA with a state marker: ' ', '+',
        // '-', or 'U'. The path is the second whitespace-delimited field.
        let line = line.trim_start_matches([' ', '+', '-', 'U']);
        let mut fields = line.split_whitespace();
        let Some(commit_sha) = fields.next() else {
            continue;
        };
        let Some(relative_path) = fields.next() else {
            continue;
        };
        let path = repo.join(relative_path);
        if !path.exists() {
            continue;
        }
        let commit_sha = git(&path, &["rev-parse", "HEAD"]).unwrap_or_else(|_| commit_sha.into());
        let staged_patch = git_output(&path, &["diff", "--cached", "--binary"])
            .ok()
            .filter(|value| !value.is_empty());
        let working_tree_patch = git_output(&path, &["diff", "--binary"])
            .ok()
            .filter(|value| !value.is_empty());
        submodules.push(SubmoduleState {
            relative_path: relative_path.into(),
            commit_sha,
            staged_patch,
            working_tree_patch,
        });
    }
    Ok(submodules)
}

pub fn capture(repo: &Path) -> Result<GitState> {
    let commit_sha = git(repo, &["rev-parse", "HEAD"])?;
    let branch_hint = git(repo, &["branch", "--show-current"])
        .ok()
        .filter(|value| !value.is_empty());
    let staged_patch = git_output(repo, &["diff", "--cached", "--binary"])
        .ok()
        .filter(|value| !value.is_empty());
    let working_tree_patch = git_output(repo, &["diff", "--binary"])
        .ok()
        .filter(|value| !value.is_empty());
    Ok(GitState {
        repository_path: repo.canonicalize()?.display().to_string(),
        commit_sha,
        branch_hint,
        staged_patch,
        working_tree_patch,
        submodules: capture_submodules(repo)?,
    })
}

fn git_with_input(repo: &Path, args: &[&str], input: &str) -> Result<()> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .as_mut()
        .expect("git stdin")
        .write_all(input.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

/// Reconstructs the captured commit and dirty state in a detached worktree.
/// The source repository's active branch is never checked out or reset.
pub fn reconstruct(state: &GitState, destination: &Path) -> Result<()> {
    let source = Path::new(&state.repository_path);
    if destination.exists() {
        bail!("destination already exists: {}", destination.display());
    }
    fs::create_dir_all(destination.parent().context("worktree parent")?)?;
    ensure_object_exists(source, &state.commit_sha)?;
    let destination_text = destination.to_str().context("non-UTF8 worktree path")?;
    git(
        source,
        &[
            "worktree",
            "add",
            "--detach",
            destination_text,
            &state.commit_sha,
        ],
    )?;
    // This is deliberately scoped to the new detached worktree. It never
    // initializes or checks out a submodule in the caller's working tree.
    git(
        destination,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "--init",
            "--recursive",
        ],
    )?;
    if let Some(patch) = &state.staged_patch {
        git_with_input(destination, &["apply", "--index", "--binary", "-"], patch)?;
    }
    if let Some(patch) = &state.working_tree_patch {
        git_with_input(destination, &["apply", "--binary", "-"], patch)?;
    }
    for submodule in &state.submodules {
        let path = destination.join(&submodule.relative_path);
        if !path.exists() {
            bail!(
                "captured submodule is missing after initialization: {}",
                submodule.relative_path
            );
        }
        ensure_object_exists(&path, &submodule.commit_sha)?;
        git(&path, &["checkout", "--detach", &submodule.commit_sha])?;
        if let Some(patch) = &submodule.staged_patch {
            git_with_input(&path, &["apply", "--index", "--binary", "-"], patch)?;
        }
        if let Some(patch) = &submodule.working_tree_patch {
            git_with_input(&path, &["apply", "--binary", "-"], patch)?;
        }
    }
    Ok(())
}

pub fn apply_patch(worktree: &Path, patch: &str) -> Result<()> {
    if patch.trim().is_empty() {
        return Ok(());
    }
    git_with_input(worktree, &["apply", "--binary", "-"], patch)
}

/// Returns a non-blocking notice when the recipient's checked-out branch has
/// advanced beyond the commit captured in the parcel. Replay still uses the
/// captured commit in an isolated detached worktree.
pub fn environment_drift(state: &GitState) -> Result<Option<String>> {
    let source = Path::new(&state.repository_path);
    let head = git(source, &["rev-parse", "HEAD"])?;
    if head == state.commit_sha {
        return Ok(None);
    }
    let count = git(
        source,
        &[
            "rev-list",
            "--count",
            &format!("{}..HEAD", state.commit_sha),
        ],
    )
    .unwrap_or_else(|_| "0".into());
    let ahead_by = count.parse::<u64>().unwrap_or(0);
    if ahead_by == 0 {
        return Ok(None);
    }
    Ok(Some(format!(
        "ENVIRONMENT_DRIFT: recipient HEAD is {ahead_by} commits ahead of parcel commit {}; verification remains pinned to the parcel commit.",
        state.commit_sha
    )))
}
