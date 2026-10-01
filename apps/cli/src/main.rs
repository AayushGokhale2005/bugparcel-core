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
    process::Command,
    thread,
    time::Duration,
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
    /// Configure BugParcel's local MCP server for coding agents in this repository.
    Mcp {
        #[command(subcommand)]
        command: McpCommands,
    },
    /// Authenticate this CLI with BugParcel Enterprise in your browser.
    Auth,
    /// Save a BugParcel Enterprise project as this workspace's remote.
    AddRemote {
        /// Full project URL, for example https://bugparcel-enterprise.vercel.app/projects/payments-api
        project_link: String,
    },
    /// Share sanitized parcel metadata with the configured Enterprise remote.
    Push {
        parcel_id: String,
        #[arg(long, default_value = "/")]
        state_pointer: String,
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

#[derive(Subcommand)]
enum McpCommands {
    /// Install a repository-scoped Codex MCP configuration so nested agent sessions can discover BugParcel.
    Install {
        /// Use Cargo to launch the MCP server from this checkout. Intended for BugParcel development only.
        #[arg(long)]
        development: bool,
        /// Parcel store to expose to agents. Defaults to <repository>/.bugparcel.
        #[arg(long)]
        store: Option<PathBuf>,
    },
    /// Confirm that Codex can discover this repository's BugParcel MCP configuration.
    Doctor,
}

fn root() -> PathBuf {
    env::var_os("BUGPARCEL_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().expect("cwd").join(".bugparcel"))
}

fn repository_root() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("locate Git repository for MCP configuration")?;
    if !output.status.success() {
        anyhow::bail!("MCP_INSTALL_REQUIRES_GIT_REPOSITORY");
    }
    Ok(PathBuf::from(String::from_utf8(output.stdout)?.trim()))
}

fn codex_config_path(repository: &Path) -> PathBuf {
    repository.join(".codex").join("config.toml")
}

fn read_codex_config(path: &Path) -> Result<toml::Table> {
    if !path.exists() {
        return Ok(toml::Table::new());
    }
    fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?
        .parse::<toml::Table>()
        .with_context(|| format!("parse {}", path.display()))
}

fn bugparcel_server_config(development: bool, store: &Path) -> Result<toml::Value> {
    let mut server = toml::Table::new();
    if development {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("Cargo.toml")
            .canonicalize()
            .context("locate BugParcel workspace Cargo.toml")?;
        server.insert("command".into(), toml::Value::String("cargo".into()));
        server.insert(
            "args".into(),
            toml::Value::Array(
                [
                    "run",
                    "--quiet",
                    "--manifest-path",
                    manifest
                        .to_str()
                        .context("BugParcel workspace path is not UTF-8")?,
                    "-p",
                    "bugparcel-mcp",
                ]
                .into_iter()
                .map(|value| toml::Value::String(value.into()))
                .collect(),
            ),
        );
    } else {
        server.insert(
            "command".into(),
            toml::Value::String("bugparcel-mcp".into()),
        );
    }
    server.insert(
        "env".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "BUGPARCEL_HOME".into(),
            toml::Value::String(store.display().to_string()),
        )])),
    );
    server.insert("startup_timeout_sec".into(), toml::Value::Integer(30));
    server.insert("tool_timeout_sec".into(), toml::Value::Integer(120));
    server.insert("required".into(), toml::Value::Boolean(true));
    Ok(toml::Value::Table(server))
}

fn run_mcp_install(development: bool, store: Option<PathBuf>) -> Result<()> {
    let repository = repository_root()?;
    let store = store
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                repository.join(path)
            }
        })
        .unwrap_or_else(|| repository.join(".bugparcel"));
    fs::create_dir_all(&store)?;
    let path = codex_config_path(&repository);
    let mut config = read_codex_config(&path)?;
    let servers = match config.entry("mcp_servers") {
        toml::map::Entry::Vacant(entry) => entry.insert(toml::Value::Table(toml::Table::new())),
        toml::map::Entry::Occupied(entry) => entry.into_mut(),
    }
    .as_table_mut()
    .context("MCP_INSTALL_FAILED: mcp_servers must be a TOML table")?;
    servers.insert(
        "bugparcel".into(),
        bugparcel_server_config(development, &store)?,
    );
    fs::create_dir_all(path.parent().expect(".codex parent"))?;
    fs::write(&path, toml::to_string_pretty(&config)?)?;
    println!("BugParcel MCP installed for {}", repository.display());
    println!("Codex config: {}", path.display());
    println!("Parcel store: {}", store.display());
    println!("Run `bugparcel mcp doctor` before assigning an agent.");
    Ok(())
}

fn run_mcp_doctor() -> Result<()> {
    let repository = repository_root()?;
    let path = codex_config_path(&repository);
    let config = read_codex_config(&path)?;
    if config
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .and_then(|servers| servers.get("bugparcel"))
        .is_none()
    {
        anyhow::bail!("MCP_UNAVAILABLE: run `bugparcel mcp install` from this repository first");
    }
    let output = Command::new("codex")
        .args(["mcp", "get", "bugparcel"])
        .current_dir(&repository)
        .output()
        .context("ask Codex for BugParcel MCP configuration")?;
    if !output.status.success() {
        anyhow::bail!("MCP_UNAVAILABLE: run `bugparcel mcp install` from this repository first");
    }
    println!("MCP_DISCOVERABLE: {}", path.display());
    Ok(())
}
fn manifest_path(root: &Path, id: &str) -> PathBuf {
    root.join("parcels").join(id).join("manifest.json")
}
fn remote_path(root: &Path) -> PathBuf {
    root.join("enterprise").join("remote.json")
}
fn session_path(root: &Path) -> PathBuf {
    root.join("enterprise").join("session.json")
}
fn enterprise_api() -> String {
    env::var("BUGPARCEL_ENTERPRISE_API")
        .unwrap_or_else(|_| "https://bugparcel-enterprise-api.ag2323.workers.dev".into())
}
fn config_value(path: &Path) -> Result<serde_json::Value> {
    Ok(serde_json::from_slice(&fs::read(path).with_context(
        || format!("missing configuration: {}", path.display()),
    )?)?)
}
fn curl_json(
    method: &str,
    url: &str,
    token: Option<&str>,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value> {
    let mut command = Command::new("curl");
    command.args([
        "--silent",
        "--show-error",
        "--fail-with-body",
        "-X",
        method,
        url,
    ]);
    command.args(["-H", "content-type: application/json"]);
    if let Some(token) = token {
        command.args(["-H", &format!("authorization: Bearer {token}")]);
    }
    if let Some(body) = body {
        command.args(["--data", &serde_json::to_string(body)?]);
    }
    let output = command
        .output()
        .context("run curl for BugParcel Enterprise")?;
    let parsed = serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap_or_else(
        |_| serde_json::json!({ "detail": String::from_utf8_lossy(&output.stderr) }),
    );
    if !output.status.success() {
        anyhow::bail!(
            "ENTERPRISE_REQUEST_FAILED: {}",
            parsed["detail"]
                .as_str()
                .or(parsed["error"].as_str())
                .unwrap_or("request failed")
        );
    }
    Ok(parsed)
}
fn session_token(root: &Path) -> Result<String> {
    if let Ok(token) = env::var("BUGPARCEL_ENTERPRISE_TOKEN") {
        return Ok(token);
    }
    config_value(&session_path(root))?["access_token"]
        .as_str()
        .map(str::to_owned)
        .context("AUTH_REQUIRED: run `bugparcel auth` first")
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
fn redact(value: &serde_json::Value, key: &str) -> serde_json::Value {
    let sensitive = [
        "password",
        "secret",
        "token",
        "authorization",
        "cookie",
        "api_key",
        "private_key",
        "credential",
    ];
    if sensitive
        .iter()
        .any(|term| key.to_ascii_lowercase().contains(term))
    {
        return serde_json::Value::String("[REDACTED]".into());
    }
    match value {
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .take(100)
                .map(|item| redact(item, ""))
                .collect(),
        ),
        serde_json::Value::Object(items) => serde_json::Value::Object(
            items
                .iter()
                .map(|(key, value)| (key.clone(), redact(value, key)))
                .collect(),
        ),
        other => other.clone(),
    }
}
fn enterprise_payload(manifest: &Manifest, pointer: &str) -> Result<serde_json::Value> {
    let state = manifest.reproduction.state.as_ref().map(|snapshot| {
        let selected = if pointer == "/" { snapshot.json.clone() } else { snapshot.json.pointer(pointer).cloned().context("STATE_POINTER_NOT_FOUND")? };
        Ok::<_, anyhow::Error>(serde_json::json!({ "source": snapshot.source, "selected_state": redact(&selected, ""), "fixtures": snapshot.fixtures.iter().map(|fixture| serde_json::json!({ "relative_path": fixture.relative_path, "sha256": fixture.sha256, "transfer": "omitted" })).collect::<Vec<_>>() }))
    }).transpose()?;
    Ok(
        serde_json::json!({ "schema_version": "enterprise-share-v1", "parcel_id": manifest.parcel_id.as_str(), "source": { "commit_sha": manifest.source.commit_sha, "branch_hint": manifest.source.branch_hint }, "status": format!("{:?}", manifest.status).to_lowercase(), "failure_assertion": redact(&serde_json::to_value(&manifest.reproduction.failure_assertion)?, ""), "environment": { "python": manifest.reproduction.environment.python, "lockfiles": manifest.reproduction.environment.lockfiles, "container_image": manifest.reproduction.environment.container_image }, "state": state, "sanitization": { "fixture_bytes": "omitted", "database_rows": "not transferred", "sensitive_values": "redacted" } }),
    )
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
        Commands::Mcp { command } => match command {
            McpCommands::Install { development, store } => run_mcp_install(development, store)?,
            McpCommands::Doctor => run_mcp_doctor()?,
        },
        Commands::Auth => {
            let request_id = format!(
                "cli_{}_{}",
                std::process::id(),
                chrono::Utc::now().timestamp_millis()
            );
            let request = curl_json(
                "POST",
                &format!("{}/api/auth/requests", enterprise_api()),
                None,
                Some(&serde_json::json!({ "request_id": request_id })),
            )?;
            let access_url = request["access_url"]
                .as_str()
                .context("AUTH_REQUEST_FAILED")?;
            println!("Open this URL and enter your invite code:\n{access_url}");
            let _ = Command::new("open").arg(access_url).spawn();
            for _ in 0..300 {
                thread::sleep(Duration::from_secs(1));
                let status = curl_json(
                    "GET",
                    &format!("{}/api/auth/requests/{request_id}", enterprise_api()),
                    None,
                    None,
                )?;
                if status["state"] == "complete" {
                    let token = status["session"]["access_token"]
                        .as_str()
                        .context("AUTH_REQUEST_FAILED")?;
                    let path = session_path(&root);
                    fs::create_dir_all(path.parent().expect("session dir"))?;
                    fs::write(
                        path,
                        serde_json::to_vec_pretty(&serde_json::json!({ "access_token": token }))?,
                    )?;
                    println!("Authenticated.");
                    return Ok(());
                }
            }
            anyhow::bail!("AUTH_TIMEOUT: no invite code was submitted within five minutes");
        }
        Commands::AddRemote { project_link } => {
            let project_link = normalize_project_link(&project_link)?;
            save_remote(&root, &project_link)?;
            println!("BugParcel remote set to {project_link}");
            println!(
                "`bugparcel push <parcel-id>` will target this project after `bugparcel auth`."
            );
        }
        Commands::Push {
            parcel_id,
            state_pointer,
        } => {
            let project_url = config_value(&remote_path(&root))?["project_url"]
                .as_str()
                .map(str::to_owned)
                .context("REMOTE_REQUIRED: run `bugparcel add-remote <project-link>` first")?;
            let token = session_token(&root)?;
            let workspace = curl_json(
                "GET",
                &format!("{}/api/workspace", enterprise_api()),
                Some(&token),
                None,
            )?;
            let project = workspace["projects"].as_array().and_then(|projects| projects.iter().find(|project| project["project_url"].as_str() == Some(project_url.as_str()))).context("PROJECT_NOT_FOUND: this account cannot access the configured BugParcel project")?;
            let manifest = load(&root, &parcel_id)?;
            let payload = enterprise_payload(&manifest, &state_pointer)?;
            let sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&payload)?));
            let response = curl_json(
                "POST",
                &format!("{}/api/parcels", enterprise_api()),
                Some(&token),
                Some(
                    &serde_json::json!({ "project_id": project["id"], "parcel_id": manifest.parcel_id.as_str(), "sha256": sha256, "status": format!("{:?}", manifest.status).to_lowercase(), "source_commit": manifest.source.commit_sha, "state_scope": state_pointer, "payload": payload }),
                ),
            )?;
            println!("{}", serde_json::to_string_pretty(&response)?);
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
