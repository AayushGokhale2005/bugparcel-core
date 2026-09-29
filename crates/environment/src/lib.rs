use anyhow::{Context, Result, bail};
use parcel_core::{EnvironmentSpec, LockfileDigest, PythonRuntime};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

const LOCKFILES: &[&str] = &[
    "requirements.txt",
    "requirements.lock",
    "poetry.lock",
    "Pipfile.lock",
    "uv.lock",
    "pyproject.toml",
];

pub fn capture(
    repository: &Path,
    command: &[String],
    environment_values: &[String],
    container_image: Option<String>,
) -> Result<EnvironmentSpec> {
    let variables = parse_environment(environment_values)?;
    let python = command
        .first()
        .filter(|program| is_python(program))
        .map(|program| {
            let output = Command::new(program)
                .arg("--version")
                .output()
                .with_context(|| format!("inspect Python runtime: {program}"))?;
            if !output.status.success() {
                bail!("Python runtime inspection failed for {program}");
            }
            Ok(PythonRuntime {
                executable: program.clone(),
                version: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            })
        })
        .transpose()?;
    let lockfiles = LOCKFILES
        .iter()
        .filter_map(|name| digest_lockfile(repository, name).transpose())
        .collect::<Result<Vec<_>>>()?;
    Ok(EnvironmentSpec {
        variables,
        python,
        lockfiles,
        container_image,
    })
}

fn is_python(program: &str) -> bool {
    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("python"))
}

fn digest_lockfile(repository: &Path, name: &str) -> Result<Option<LockfileDigest>> {
    let path = repository.join(name);
    if !path.is_file() {
        return Ok(None);
    }
    let contents = fs::read(&path).with_context(|| format!("read lockfile: {}", path.display()))?;
    let sha256 = format!("{:x}", Sha256::digest(contents));
    Ok(Some(LockfileDigest {
        path: name.into(),
        sha256,
    }))
}

fn parse_environment(values: &[String]) -> Result<BTreeMap<String, String>> {
    values
        .iter()
        .map(|value| {
            let (key, value) = value
                .split_once('=')
                .context("environment values must use KEY=VALUE")?;
            if key.is_empty() {
                bail!("environment variable name must not be empty");
            }
            Ok((key.to_owned(), value.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_allowlisted_environment_values() {
        let environment = parse_environment(&["LOG_LEVEL=debug".into()]).unwrap();
        assert_eq!(environment["LOG_LEVEL"], "debug");
    }
}
