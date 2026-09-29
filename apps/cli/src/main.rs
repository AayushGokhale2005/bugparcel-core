use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use parcel_core::{FailureAssertion, Manifest, ParcelStatus, ReproductionSpec, StateSnapshot};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(name = "bugparcel", about = "Local-first reproducible failure parcels")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Capture {
        #[arg(long)]
        name: String,
        #[arg(long = "expect-output")]
        expected_output_contains: Vec<String>,
        #[arg(long, default_value_t = 1)]
        expected_exit_code: i32,
        #[arg(long)]
        contract_file: Option<PathBuf>,
        #[arg(long = "env")]
        environment_values: Vec<String>,
        #[arg(long)]
        docker_image: Option<String>,
        #[arg(long)]
        state_file: Option<PathBuf>,
        #[arg(long, default_value = "/")]
        state_json_pointer: String,
        #[arg(required = true, trailing_var_arg = true)]
        command: Vec<String>,
    },
    Show {
        parcel_id: String,
    },
    Reproduce {
        parcel_id: String,
    },
    Verify {
        parcel_id: String,
        #[arg(long)]
        patch: PathBuf,
    },
    Reduce {
        parcel_id: String,
        #[arg(long)]
        output: PathBuf,
    },
}

fn root() -> PathBuf {
    env::var_os("BUGPARCEL_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().expect("cwd").join(".bugparcel"))
}
fn manifest_path(root: &Path, id: &str) -> PathBuf {
    root.join("parcels").join(id).join("manifest.json")
}
fn load(root: &Path, id: &str) -> Result<Manifest> {
    Ok(serde_json::from_slice(&fs::read(manifest_path(root, id))?)?)
}
fn save(root: &Path, manifest: &Manifest) -> Result<()> {
    let path = manifest_path(root, manifest.parcel_id.as_str());
    fs::create_dir_all(path.parent().expect("parcel dir"))?;
    fs::write(path, serde_json::to_vec_pretty(manifest)?)?;
    Ok(())
}
fn worktree_path(root: &Path, parcel_id: &str, purpose: &str) -> PathBuf {
    root.join("worktrees")
        .join(parcel_id)
        .join(format!(
            "{purpose}-{}",
            chrono::Utc::now().timestamp_millis()
        ))
        .join("repo")
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = root();
    match cli.command {
        Commands::Capture {
            name,
            expected_output_contains,
            expected_exit_code,
            contract_file,
            environment_values,
            docker_image,
            state_file,
            state_json_pointer,
            command,
        } => {
            let repo = env::current_dir()?;
            let state = git_state::capture(&repo).context("capture Git state")?;
            let mut manifest = Manifest::new(
                state,
                ReproductionSpec {
                    environment: environment::capture(
                        &repo,
                        &command,
                        &environment_values,
                        docker_image,
                    )?,
                    command,
                    failure_assertion: FailureAssertion {
                        expected_exit_code,
                        expected_output_contains,
                        context: contract_file
                            .map(fs::read)
                            .transpose()?
                            .map(|bytes| serde_json::from_slice(&bytes))
                            .transpose()?,
                    },
                    state: state_file
                        .map(|path| {
                            let document: serde_json::Value =
                                serde_json::from_slice(&fs::read(&path)?)?;
                            let json = document
                                .pointer(&state_json_pointer)
                                .cloned()
                                .with_context(|| {
                                    format!("state JSON pointer not found: {state_json_pointer}")
                                })?;
                            Ok::<StateSnapshot, anyhow::Error>(StateSnapshot {
                                source: path.display().to_string(),
                                json,
                            })
                        })
                        .transpose()?,
                },
            );
            manifest.transition(
                ParcelStatus::Captured,
                Some(format!("captured by CLI as {name}")),
            )?;
            save(&root, &manifest)?;
            println!("{} captured", manifest.parcel_id.as_str());
        }
        Commands::Show { parcel_id } => println!(
            "{}",
            serde_json::to_string_pretty(&load(&root, &parcel_id)?)?
        ),
        Commands::Reproduce { parcel_id } => {
            let mut manifest = load(&root, &parcel_id)?;
            manifest.transition(ParcelStatus::Reproducing, None)?;
            let worktree = worktree_path(&root, &parcel_id, "replay");
            git_state::reconstruct(&manifest.source, &worktree)?;
            let result = replay::run(&manifest.reproduction, &worktree)?;
            manifest.transition(
                if result.matched {
                    ParcelStatus::Reproducible
                } else {
                    ParcelStatus::ReproFailed
                },
                Some(format!("observed exit code {}", result.observed_exit_code)),
            )?;
            save(&root, &manifest)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Commands::Verify { parcel_id, patch } => {
            let mut manifest = load(&root, &parcel_id)?;
            if manifest.status != ParcelStatus::Reproducible {
                anyhow::bail!("VERIFY_REQUIRES_REPRODUCIBLE_PARCEL");
            }
            manifest.transition(
                ParcelStatus::FixProposed,
                Some(format!("candidate patch: {}", patch.display())),
            )?;
            manifest.transition(ParcelStatus::Verifying, None)?;
            let worktree = worktree_path(&root, &parcel_id, "verify");
            git_state::reconstruct(&manifest.source, &worktree)?;
            git_state::apply_patch(&worktree, &fs::read_to_string(patch)?)?;
            let status = verify::verify_patch(&manifest.reproduction, &worktree)?;
            let (next, message) = match status {
                verify::VerificationStatus::Verified => {
                    (ParcelStatus::Verified, "original failure no longer matched")
                }
                verify::VerificationStatus::OriginalFailureRemains => {
                    (ParcelStatus::VerifyFailed, "original failure still matched")
                }
            };
            manifest.transition(next, Some(message.into()))?;
            save(&root, &manifest)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }
        Commands::Reduce { parcel_id, output } => {
            let manifest = load(&root, &parcel_id)?;
            if manifest.status != ParcelStatus::Reproducible {
                anyhow::bail!("REDUCE_REQUIRES_REPRODUCIBLE_PARCEL");
            }
            let state = manifest
                .reproduction
                .state
                .as_ref()
                .context("REDUCE_REQUIRES_CAPTURED_JSON_STATE")?;
            if manifest.reproduction.environment.container_image.is_some() {
                anyhow::bail!("JSON_REDUCTION_WITH_DOCKER_IS_NOT_IMPLEMENTED_YET");
            }
            let worktree = worktree_path(&root, &parcel_id, "reduce");
            git_state::reconstruct(&manifest.source, &worktree)?;
            let minimized = reducer::minimize(state.json.clone(), |candidate| {
                replay::run_with_environment(
                    &manifest.reproduction,
                    &worktree,
                    &[("BUGPARCEL_STATE_JSON".into(), candidate.to_string())],
                )
                .is_ok_and(|result| result.matched)
            });
            fs::write(&output, serde_json::to_vec_pretty(&minimized)?)?;
            println!("{}", serde_json::to_string_pretty(&minimized)?);
        }
    }
    Ok(())
}
