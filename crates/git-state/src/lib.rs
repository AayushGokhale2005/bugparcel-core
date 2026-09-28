use anyhow::{Context, Result, bail};
use parcel_core::GitState;
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
    if let Some(patch) = &state.staged_patch {
        git_with_input(destination, &["apply", "--index", "--binary", "-"], patch)?;
    }
    if let Some(patch) = &state.working_tree_patch {
        git_with_input(destination, &["apply", "--binary", "-"], patch)?;
    }
    Ok(())
}

pub fn apply_patch(worktree: &Path, patch: &str) -> Result<()> {
    if patch.trim().is_empty() {
        return Ok(());
    }
    git_with_input(worktree, &["apply", "--binary", "-"], patch)
}
