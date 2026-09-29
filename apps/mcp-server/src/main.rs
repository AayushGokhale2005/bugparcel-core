use anyhow::{Context, Result};
use parcel_core::{FailureAssertion, Manifest, ParcelStatus, ReproductionSpec, StateSnapshot};
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};

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

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    })
}

fn tool_definitions() -> Value {
    json!([
        tool(
            "bugparcel_list_parcels",
            "List local BugParcel manifests available to this agent.",
            json!({"type": "object", "properties": {}}),
        ),
        tool(
            "bugparcel_get_parcel",
            "Read the complete, append-only manifest for one parcel.",
            json!({
                "type": "object",
                "properties": {"parcel_id": {"type": "string"}},
                "required": ["parcel_id"],
            }),
        ),
        tool(
            "bugparcel_get_contract",
            "Read only the replay contract: command, expected failure, captured state, fixtures, and environment.",
            json!({
                "type": "object",
                "properties": {"parcel_id": {"type": "string"}},
                "required": ["parcel_id"],
            }),
        ),
        tool(
            "bugparcel_reproduce",
            "Reproduce a captured failure in a fresh detached Git worktree. This never changes the source branch.",
            json!({
                "type": "object",
                "properties": {"parcel_id": {"type": "string"}},
                "required": ["parcel_id"],
            }),
        ),
        tool(
            "bugparcel_verify",
            "Apply a candidate patch in a fresh detached worktree and verify both failure removal and the immutable acceptance contract.",
            json!({
                "type": "object",
                "properties": {
                    "parcel_id": {"type": "string"},
                    "patch_path": {"type": "string"},
                },
                "required": ["parcel_id", "patch_path"],
            }),
        ),
        tool(
            "bugparcel_capture",
            "Capture a failure from a local Git repository. The command runs later only in detached worktrees; capture itself does not execute it.",
            json!({
                "type": "object",
                "properties": {
                    "repository_path": {"type": "string", "description": "Absolute local Git repository path."},
                    "command": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                    "expected_output_contains": {"type": "array", "items": {"type": "string"}},
                    "expected_exit_code": {"type": "integer", "default": 1},
                    "environment": {"type": "object", "additionalProperties": {"type": "string"}},
                    "docker_image": {"type": "string"},
                    "state": {"description": "Optional JSON state supplied to replay.", "type": "object"},
                    "state_source": {"type": "string"},
                },
                "required": ["repository_path", "command"],
            }),
        ),
        tool(
            "bugparcel_reduce",
            "Minimize captured JSON state while preserving the failure contract. Returns the reduced state without editing the source repository.",
            json!({
                "type": "object",
                "properties": {"parcel_id": {"type": "string"}},
                "required": ["parcel_id"],
            }),
        ),
        tool(
            "bugparcel_cleanup_worktrees",
            "Remove only detached worktrees created by this local BugParcel store. Source repositories are never touched.",
            json!({
                "type": "object",
                "properties": {"parcel_id": {"type": "string"}},
            }),
        ),
        tool(
            "bugparcel_diagnose",
            "Re-run an incident safely and return an agent-ready diagnosis bundle: failure contract, captured state, replay result, and isolated worktree location.",
            json!({
                "type": "object",
                "properties": {"parcel_id": {"type": "string"}},
                "required": ["parcel_id"],
            }),
        ),
        tool(
            "bugparcel_propose_fix",
            "Verify a unified Git diff supplied by an agent. The diff is applied only in a new detached BugParcel worktree and is never applied to the source branch.",
            json!({
                "type": "object",
                "properties": {
                    "parcel_id": {"type": "string"},
                    "patch": {"type": "string", "description": "A unified Git diff."},
                },
                "required": ["parcel_id", "patch"],
            }),
        ),
        tool(
            "bugparcel_export_pytest",
            "Export a verified parcel as a standalone pytest regression test.",
            json!({
                "type": "object",
                "properties": {"parcel_id": {"type": "string"}, "output_dir": {"type": "string"}},
                "required": ["parcel_id", "output_dir"],
            }),
        ),
    ])
}

fn text_result(value: Value) -> Value {
    json!({
        "content": [{"type": "text", "text": serde_json::to_string_pretty(&value).unwrap()}],
        "structuredContent": value,
    })
}

fn argument<'a>(arguments: &'a Value, key: &str) -> Result<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("missing required tool argument: {key}"))
}

fn string_list(arguments: &Value, key: &str) -> Result<Vec<String>> {
    let values: Vec<String> = arguments
        .get(key)
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()?
        .unwrap_or_default();
    Ok(values)
}

fn string_map(arguments: &Value, key: &str) -> Result<std::collections::BTreeMap<String, String>> {
    Ok(arguments
        .get(key)
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()?
        .unwrap_or_default())
}

fn list_parcels(root: &Path) -> Result<Value> {
    let directory = root.join("parcels");
    if !directory.exists() {
        return Ok(json!([]));
    }
    let mut parcels = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path().join("manifest.json");
        if !path.exists() {
            continue;
        }
        let manifest: Manifest = serde_json::from_slice(&fs::read(path)?)?;
        parcels.push(json!({
            "parcel_id": manifest.parcel_id.as_str(),
            "status": manifest.status,
            "created_at": manifest.created_at,
            "branch_hint": manifest.source.branch_hint,
            "commit_sha": manifest.source.commit_sha,
        }));
    }
    parcels.sort_by_key(|value| value["created_at"].as_str().map(str::to_owned));
    Ok(Value::Array(parcels))
}

fn contract(root: &Path, parcel_id: &str) -> Result<Value> {
    let manifest = load(root, parcel_id)?;
    Ok(json!({
        "parcel_id": manifest.parcel_id.as_str(),
        "status": manifest.status,
        "command": manifest.reproduction.command,
        "failure_assertion": manifest.reproduction.failure_assertion,
        "environment": manifest.reproduction.environment,
        "state": manifest.reproduction.state,
    }))
}

fn capture(root: &Path, arguments: &Value) -> Result<Value> {
    let repository = PathBuf::from(argument(arguments, "repository_path")?);
    if !repository.is_absolute() {
        anyhow::bail!("repository_path must be absolute");
    }
    let command = string_list(arguments, "command")?;
    if command.is_empty() {
        anyhow::bail!("command must contain at least one executable");
    }
    let environment_values = string_map(arguments, "environment")?
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>();
    let state = arguments.get("state").cloned().map(|json| StateSnapshot {
        source: arguments
            .get("state_source")
            .and_then(Value::as_str)
            .unwrap_or("agent-provided JSON")
            .to_owned(),
        json,
        fixtures: vec![],
    });
    let source = git_state::capture(&repository).context("capture Git state")?;
    let mut reproduction = ReproductionSpec {
        environment: environment::capture(
            &repository,
            &command,
            &environment_values,
            arguments
                .get("docker_image")
                .and_then(Value::as_str)
                .map(str::to_owned),
        )?,
        command,
        failure_assertion: FailureAssertion {
            expected_exit_code: arguments
                .get("expected_exit_code")
                .and_then(Value::as_i64)
                .unwrap_or(1) as i32,
            expected_output_contains: string_list(arguments, "expected_output_contains")?,
            context: None,
        },
        state,
        immutable_inputs: vec![],
    };
    reproduction.immutable_inputs = verify::capture_immutable_inputs(&repository, &reproduction)?;
    let mut manifest = Manifest::new(source, reproduction);
    manifest.transition(
        ParcelStatus::Captured,
        Some("captured through local MCP".into()),
    )?;
    save(root, &manifest)?;
    Ok(serde_json::to_value(manifest)?)
}

fn reproduce(root: &Path, parcel_id: &str) -> Result<Value> {
    let mut manifest = load(root, parcel_id)?;
    let should_track_transition = matches!(
        manifest.status,
        ParcelStatus::Captured | ParcelStatus::Reducing
    );
    if should_track_transition {
        manifest.transition(
            ParcelStatus::Reproducing,
            Some("requested through local MCP".into()),
        )?;
    }
    let worktree = worktree_path(root, parcel_id, "mcp-replay");
    git_state::reconstruct(&manifest.source, &worktree)?;
    let result = replay::run(&manifest.reproduction, &worktree)?;
    if should_track_transition {
        manifest.transition(
            if result.matched {
                ParcelStatus::Reproducible
            } else {
                ParcelStatus::ReproFailed
            },
            Some(format!("observed exit code {}", result.observed_exit_code)),
        )?;
        save(root, &manifest)?;
    }
    let environment_drift = git_state::environment_drift(&manifest.source)?;
    Ok(
        json!({"parcel": manifest, "replay": result, "worktree": worktree, "advisories": environment_drift.into_iter().collect::<Vec<_>>() }),
    )
}

fn diagnosis(root: &Path, parcel_id: &str) -> Result<Value> {
    let baseline = reproduce(root, parcel_id)?;
    let contract = contract(root, parcel_id)?;
    Ok(json!({
        "parcel_id": parcel_id,
        "diagnosis": {
            "contract": contract,
            "baseline": baseline["replay"].clone(),
            "isolated_worktree": baseline["worktree"].clone(),
            "advisories": baseline["advisories"].clone(),
        },
        "next_action": "Inspect the isolated worktree, then send a unified diff to bugparcel_propose_fix. The source branch remains unchanged.",
    }))
}

fn changed_files(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|line| line.strip_prefix("+++ b/"))
        .filter(|path| *path != "/dev/null")
        .map(str::to_owned)
        .collect()
}

fn propose_fix(root: &Path, parcel_id: &str, patch: &str) -> Result<Value> {
    let mut manifest = load(root, parcel_id)?;
    if manifest.status != ParcelStatus::Reproducible {
        anyhow::bail!("VERIFY_REQUIRES_REPRODUCIBLE_PARCEL");
    }
    manifest.transition(
        ParcelStatus::FixProposed,
        Some("candidate patch submitted through local MCP".into()),
    )?;
    manifest.transition(
        ParcelStatus::Verifying,
        Some("verifying agent-supplied unified diff in detached worktree".into()),
    )?;
    let worktree = worktree_path(root, parcel_id, "mcp-agent-fix");
    git_state::reconstruct(&manifest.source, &worktree)?;
    git_state::apply_patch(&worktree, patch)?;
    let outcome = verify::verify_patch(&manifest.reproduction, &worktree)?;
    let advisories = git_state::environment_drift(&manifest.source)?;
    let (next, verification, detail) = match outcome {
        verify::VerificationStatus::Verified => (ParcelStatus::Verified, "verified", None),
        verify::VerificationStatus::OriginalFailureRemains => {
            (ParcelStatus::VerifyFailed, "original_failure_remains", None)
        }
        verify::VerificationStatus::AcceptanceFailed { reason } => {
            (ParcelStatus::VerifyFailed, "verify_failed", Some(reason))
        }
        verify::VerificationStatus::ImmutableContractChanged { path } => {
            (ParcelStatus::VerifyFailed, "verify_failed", Some(path))
        }
    };
    manifest.transition(
        next,
        Some(format!("agent fix verification outcome: {verification}")),
    )?;
    save(root, &manifest)?;
    Ok(json!({
        "parcel_id": parcel_id,
        "verification": verification,
        "changed_files": changed_files(patch),
        "isolated_worktree": worktree,
        "source_branch_modified": false,
        "verification_detail": detail,
        "advisories": advisories.into_iter().collect::<Vec<_>>(),
        "next_action": if verification == "verified" {
            "Review and apply this same diff yourself when ready. BugParcel did not modify the source branch."
        } else {
            "Revise the diff and retry with a new reproducible parcel."
        },
    }))
}

fn verify(root: &Path, parcel_id: &str, patch_path: &Path) -> Result<Value> {
    let mut manifest = load(root, parcel_id)?;
    if manifest.status != ParcelStatus::Reproducible {
        anyhow::bail!("VERIFY_REQUIRES_REPRODUCIBLE_PARCEL");
    }
    manifest.transition(
        ParcelStatus::FixProposed,
        Some(format!("candidate patch: {}", patch_path.display())),
    )?;
    manifest.transition(
        ParcelStatus::Verifying,
        Some("requested through local MCP".into()),
    )?;
    let worktree = worktree_path(root, parcel_id, "mcp-verify");
    git_state::reconstruct(&manifest.source, &worktree)?;
    git_state::apply_patch(&worktree, &fs::read_to_string(patch_path)?)?;
    let outcome = verify::verify_patch(&manifest.reproduction, &worktree)?;
    let advisories = git_state::environment_drift(&manifest.source)?;
    let (next, outcome_text, detail) = match outcome {
        verify::VerificationStatus::Verified => (ParcelStatus::Verified, "verified", None),
        verify::VerificationStatus::OriginalFailureRemains => {
            (ParcelStatus::VerifyFailed, "original_failure_remains", None)
        }
        verify::VerificationStatus::AcceptanceFailed { reason } => {
            (ParcelStatus::VerifyFailed, "verify_failed", Some(reason))
        }
        verify::VerificationStatus::ImmutableContractChanged { path } => {
            (ParcelStatus::VerifyFailed, "verify_failed", Some(path))
        }
    };
    manifest.transition(
        next,
        Some(format!("MCP verification outcome: {outcome_text}")),
    )?;
    save(root, &manifest)?;
    Ok(
        json!({"parcel": manifest, "verification": outcome_text, "verification_detail": detail, "advisories": advisories.into_iter().collect::<Vec<_>>(), "worktree": worktree}),
    )
}

fn reduce(root: &Path, parcel_id: &str) -> Result<Value> {
    let manifest = load(root, parcel_id)?;
    if manifest.status != ParcelStatus::Reproducible {
        anyhow::bail!("REDUCE_REQUIRES_REPRODUCIBLE_PARCEL");
    }
    if manifest.reproduction.environment.container_image.is_some() {
        anyhow::bail!("JSON_REDUCTION_WITH_DOCKER_IS_NOT_IMPLEMENTED_YET");
    }
    let state = manifest
        .reproduction
        .state
        .as_ref()
        .context("REDUCE_REQUIRES_CAPTURED_JSON_STATE")?;
    let worktree = worktree_path(root, parcel_id, "mcp-reduce");
    git_state::reconstruct(&manifest.source, &worktree)?;
    let minimized = reducer::minimize(state.json.clone(), |candidate| {
        replay::run_with_environment(
            &manifest.reproduction,
            &worktree,
            &[("BUGPARCEL_STATE_JSON".into(), candidate.to_string())],
        )
        .is_ok_and(|result| result.matched)
    });
    Ok(json!({
        "parcel_id": parcel_id,
        "state_source": state.source,
        "reduced_state": minimized,
        "worktree": worktree,
    }))
}

fn cleanup_worktrees(root: &Path, parcel_id: Option<&str>) -> Result<Value> {
    let worktrees = root.join("worktrees");
    if !worktrees.exists() {
        return Ok(json!({"removed": []}));
    }
    let targets = if let Some(parcel_id) = parcel_id {
        vec![worktrees.join(parcel_id)]
    } else {
        fs::read_dir(&worktrees)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect()
    };
    let mut removed = Vec::new();
    for target in targets {
        if target.exists() {
            fs::remove_dir_all(&target)?;
            removed.push(target);
        }
    }
    Ok(json!({"removed": removed}))
}

fn export_pytest(root: &Path, parcel_id: &str, output_dir: &Path) -> Result<Value> {
    let manifest = load(root, parcel_id)?;
    if manifest.status != ParcelStatus::Verified {
        anyhow::bail!("EXPORT_REQUIRES_VERIFIED_PARCEL");
    }
    fs::create_dir_all(output_dir)?;
    let path = output_dir.join(format!("test_{}.py", manifest.parcel_id.as_str()));
    let command = serde_json::to_string(&manifest.reproduction.command)?;
    let test_name = manifest.parcel_id.as_str().replace('-', "_");
    fs::write(
        &path,
        format!(
            "# Generated by BugParcel from an immutable verified parcel.\n# Run with: BUGPARCEL_REPOSITORY=/path/to/checkout pytest {}\n\nimport os\nimport subprocess\nfrom pathlib import Path\n\n\ndef test_{test_name}_regression():\n    repo = Path(os.environ[\"BUGPARCEL_REPOSITORY\"])\n    result = subprocess.run({command}, cwd=repo, text=True, capture_output=True)\n    assert result.returncode == 0, result.stdout + result.stderr\n",
            path.display(),
        ),
    )?;
    Ok(
        json!({"parcel_id": parcel_id, "pytest_path": path, "command": manifest.reproduction.command}),
    )
}

fn call_tool(root: &Path, name: &str, arguments: &Value) -> Result<Value> {
    match name {
        "bugparcel_list_parcels" => Ok(text_result(list_parcels(root)?)),
        "bugparcel_get_parcel" => Ok(text_result(serde_json::to_value(load(
            root,
            argument(arguments, "parcel_id")?,
        )?)?)),
        "bugparcel_get_contract" => Ok(text_result(contract(
            root,
            argument(arguments, "parcel_id")?,
        )?)),
        "bugparcel_capture" => Ok(text_result(capture(root, arguments)?)),
        "bugparcel_reproduce" => Ok(text_result(reproduce(
            root,
            argument(arguments, "parcel_id")?,
        )?)),
        "bugparcel_verify" => Ok(text_result(verify(
            root,
            argument(arguments, "parcel_id")?,
            Path::new(argument(arguments, "patch_path")?),
        )?)),
        "bugparcel_reduce" => Ok(text_result(reduce(
            root,
            argument(arguments, "parcel_id")?,
        )?)),
        "bugparcel_cleanup_worktrees" => Ok(text_result(cleanup_worktrees(
            root,
            arguments.get("parcel_id").and_then(Value::as_str),
        )?)),
        "bugparcel_diagnose" => Ok(text_result(diagnosis(
            root,
            argument(arguments, "parcel_id")?,
        )?)),
        "bugparcel_propose_fix" => Ok(text_result(propose_fix(
            root,
            argument(arguments, "parcel_id")?,
            argument(arguments, "patch")?,
        )?)),
        "bugparcel_export_pytest" => Ok(text_result(export_pytest(
            root,
            argument(arguments, "parcel_id")?,
            Path::new(argument(arguments, "output_dir")?),
        )?)),
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: impl ToString) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.to_string()}})
}

fn dispatch(root: &Path, request: Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "bugparcel", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Local-only BugParcel server. Reproduce and verify always run in detached Git worktrees.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tool_definitions()})),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .context("missing tool name");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            name.and_then(|name| call_tool(root, name, &arguments))
        }
        "notifications/initialized" | "notifications/cancelled" => return None,
        _ => Err(anyhow::anyhow!("method not found: {method}")),
    };
    id.map(|id| match result {
        Ok(value) => response(id, value),
        Err(error_value) => error(id, -32000, error_value),
    })
}

fn main() -> Result<()> {
    let root = root();
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(parse_error) => {
                writeln!(stdout, "{}", error(Value::Null, -32700, parse_error))?;
                stdout.flush()?;
                continue;
            }
        };
        if let Some(message) = dispatch(&root, request) {
            writeln!(stdout, "{}", serde_json::to_string(&message)?)?;
            stdout.flush()?;
        }
    }
    Ok(())
}
