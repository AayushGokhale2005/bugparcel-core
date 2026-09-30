use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use clap::{Parser, Subcommand};
use parcel_core::{
    FailureAssertion, FixtureSnapshot, Manifest, ParcelStatus, ReproductionSpec, StateSnapshot,
};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    name = "bugparcel",
    about = "Local-first reproducible failure parcels",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Save a BugParcel Enterprise project as this workspace's remote.
    AddRemote {
        /// Full project URL, for example https://bugparcel-enterprise.vercel.app/projects/payments-api
        project_link: String,
    },
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
        #[arg(long = "fixture-file")]
        fixture_files: Vec<PathBuf>,
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
    Export {
        parcel_id: String,
        #[arg(long)]
        pytest: PathBuf,
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
fn remote_path(root: &Path) -> PathBuf {
    root.join("enterprise").join("remote.json")
}
fn normalize_project_link(value: &str) -> Result<String> {
    let project_link = value.trim().trim_end_matches('/');
    let remainder = project_link
        .strip_prefix("https://")
        .or_else(|| project_link.strip_prefix("http://"))
        .context("PROJECT_LINK_INVALID: paste the full BugParcel project URL")?;
    let (_, path) = remainder
        .split_once('/')
        .context("PROJECT_LINK_INVALID: expected .../projects/<project>")?;
    let parts = path
        .split('?')
        .next()
        .unwrap_or_default()
        .split('/')
        .collect::<Vec<_>>();
    if parts.len() != 2
        || parts[0] != "projects"
        || parts[1].is_empty()
        || parts[1].chars().any(char::is_whitespace)
    {
        anyhow::bail!(
            "PROJECT_LINK_INVALID: expected https://bugparcel-enterprise.vercel.app/projects/<project>"
        );
    }
    Ok(project_link.to_owned())
}
fn save_remote(root: &Path, project_link: &str) -> Result<()> {
    let path = remote_path(root);
    fs::create_dir_all(path.parent().expect("enterprise config dir"))?;
    fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({ "project_url": project_link }))?,
    )?;
    Ok(())
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

fn capture_fixtures(repo: &Path, fixture_files: &[PathBuf]) -> Result<Vec<FixtureSnapshot>> {
    fixture_files
        .iter()
        .map(|path| {
            let relative_path = path.strip_prefix(repo).with_context(|| {
                format!("fixture must be inside repository: {}", path.display())
            })?;
            if relative_path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                anyhow::bail!(
                    "fixture path must not escape repository: {}",
                    path.display()
                );
            }
            let contents = fs::read(path)?;
            Ok(FixtureSnapshot {
                relative_path: relative_path.display().to_string(),
                sha256: format!("{:x}", Sha256::digest(&contents)),
                contents_base64: BASE64.encode(contents),
            })
        })
        .collect()
}

fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    let root = root();
    match cli.command {
        Commands::AddRemote { project_link } => {
            let project_link = normalize_project_link(&project_link)?;
            save_remote(&root, &project_link)?;
            println!("BugParcel remote set to {project_link}");
            println!("Future `bugparcel push <parcel-id>` commands will target this project.");
        }
        Commands::Capture {
            name,
            expected_output_contains,
            expected_exit_code,
            contract_file,
            environment_values,
            docker_image,
            state_file,
            state_json_pointer,
            fixture_files,
            command,
        } => {
            let repo = env::current_dir()?;
            let state = git_state::capture(&repo).context("capture Git state")?;
            let fixtures = capture_fixtures(&repo, &fixture_files)?;
            let mut reproduction = ReproductionSpec {
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
                            fixtures: fixtures.clone(),
                        })
                    })
                    .transpose()?
                    .or_else(|| {
                        (!fixtures.is_empty()).then(|| StateSnapshot {
                            source: "fixture-only".into(),
                            json: serde_json::Value::Null,
                            fixtures,
                        })
                    }),
                immutable_inputs: vec![],
            };
            reproduction.immutable_inputs = verify::capture_immutable_inputs(&repo, &reproduction)?;
            let mut manifest = Manifest::new(state, reproduction);
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
                verify::VerificationStatus::Verified => (
                    ParcelStatus::Verified,
                    "failure removed and acceptance contract passed",
                ),
                verify::VerificationStatus::OriginalFailureRemains => {
                    (ParcelStatus::VerifyFailed, "original failure still matched")
                }
                verify::VerificationStatus::AcceptanceFailed { .. } => (
                    ParcelStatus::VerifyFailed,
                    "VERIFY_FAILED: acceptance contract failed",
                ),
                verify::VerificationStatus::ImmutableContractChanged { .. } => (
                    ParcelStatus::VerifyFailed,
                    "VERIFY_FAILED: immutable acceptance contract changed",
                ),
            };
            manifest.transition(next, Some(message.into()))?;
            save(&root, &manifest)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            if next == ParcelStatus::VerifyFailed {
                anyhow::bail!("VERIFY_FAILED");
            }
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
        Commands::Export { parcel_id, pytest } => {
            let manifest = load(&root, &parcel_id)?;
            if manifest.status != ParcelStatus::Verified {
                anyhow::bail!("EXPORT_REQUIRES_VERIFIED_PARCEL");
            }
            export_pytest(&manifest, &pytest)?;
            println!("{}", pytest.display());
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run_cli() {
        eprintln!("{error:#}");
        let message = error.to_string();
        let exit_code = if message.contains("REPRO_MISSING_GIT_OBJECT") {
            32
        } else if message.contains("VERIFY_FAILED") {
            21
        } else {
            1
        };
        std::process::exit(exit_code);
    }
}

fn export_pytest(manifest: &Manifest, output: &Path) -> Result<()> {
    fs::create_dir_all(output)?;
    let test_path = output.join(format!("test_{}.py", manifest.parcel_id.as_str()));
    let command = serde_json::to_string(&manifest.reproduction.command)?;
    let body = format!(
        "# Generated by BugParcel from a verified, immutable replay contract.\n# Run with: BUGPARCEL_REPOSITORY=/path/to/checkout pytest {}\n\nimport os\nimport subprocess\nfrom pathlib import Path\n\n\ndef test_{}_regression():\n    repo = Path(os.environ[\"BUGPARCEL_REPOSITORY\"])\n    result = subprocess.run({}, cwd=repo, text=True, capture_output=True)\n    assert result.returncode == 0, result.stdout + result.stderr\n",
        test_path.display(),
        manifest.parcel_id.as_str().replace('-', "_"),
        command
    );
    fs::write(test_path, body)?;
    Ok(())
}
