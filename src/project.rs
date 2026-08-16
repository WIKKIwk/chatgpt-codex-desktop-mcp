use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use tokio::fs;

#[derive(Debug, Clone, Copy, Default, Deserialize, schemars::JsonSchema)]
pub enum ProjectCheckKind {
    #[serde(rename = "auto")]
    #[default]
    Auto,
    #[serde(rename = "test")]
    Test,
    #[serde(rename = "check")]
    Check,
    #[serde(rename = "typecheck")]
    Typecheck,
    #[serde(rename = "lint")]
    Lint,
    #[serde(rename = "build")]
    Build,
    #[serde(rename = "format-check")]
    FormatCheck,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCheck {
    pub command: String,
    pub args: Vec<String>,
    pub label: String,
}

pub async fn detect_project_type(root: &Path) -> String {
    let markers = [
        ("pubspec.yaml", "flutter/dart"),
        ("Cargo.toml", "rust"),
        ("package.json", "node"),
        ("pyproject.toml", "python"),
        ("requirements.txt", "python"),
        ("go.mod", "go"),
    ];
    for (marker, project_type) in markers {
        if fs::metadata(root.join(marker))
            .await
            .is_ok_and(|metadata| metadata.is_file())
        {
            return project_type.to_owned();
        }
    }
    "generic".to_owned()
}

pub async fn select_project_check(
    root: &Path,
    kind: ProjectCheckKind,
) -> Result<ProjectCheck, String> {
    let project_type = detect_project_type(root).await;
    match project_type.as_str() {
        "node" => select_node_check(root, kind).await,
        "rust" => Ok(select_rust_check(kind)),
        "flutter/dart" => select_flutter_check(kind),
        "python"
            if matches!(
                kind,
                ProjectCheckKind::Auto | ProjectCheckKind::Test | ProjectCheckKind::Check
            ) =>
        {
            Ok(ProjectCheck {
                command: "pytest".to_owned(),
                args: Vec::new(),
                label: "Running pytest".to_owned(),
            })
        }
        _ => Err(format!(
            "No safe automatic '{}' check is defined for project type '{}'. Use run_project_command with an explicit allowed command.",
            kind.as_str(),
            project_type
        )),
    }
}

async fn select_node_check(root: &Path, kind: ProjectCheckKind) -> Result<ProjectCheck, String> {
    let package_path = root.join("package.json");
    let package_text = fs::read_to_string(&package_path)
        .await
        .map_err(|error| format!("Could not read package.json: {error}"))?;
    let package: Value = serde_json::from_str(&package_text)
        .map_err(|error| format!("Could not parse package.json: {error}"))?;
    let scripts = package
        .get("scripts")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let candidates = node_candidates(kind);
    let script = candidates
        .iter()
        .find(|name| scripts.get(**name).and_then(Value::as_str).is_some())
        .copied()
        .ok_or_else(|| {
            let available = scripts.keys().cloned().collect::<Vec<_>>();
            format!(
                "No safe package.json script found for '{}'. Available scripts: {}",
                kind.as_str(),
                if available.is_empty() {
                    "(none)".to_owned()
                } else {
                    available.join(", ")
                }
            )
        })?;
    let manager = detect_node_package_manager(root).await;
    let args = if manager == "npm" && script == "test" {
        vec!["test".to_owned()]
    } else {
        vec!["run".to_owned(), script.to_owned()]
    };
    Ok(ProjectCheck {
        label: format!("Running {} {}", manager, args.join(" ")),
        command: manager,
        args,
    })
}

fn select_rust_check(kind: ProjectCheckKind) -> ProjectCheck {
    let args = match kind {
        ProjectCheckKind::Auto | ProjectCheckKind::Check | ProjectCheckKind::Typecheck => {
            vec!["check"]
        }
        ProjectCheckKind::Test => vec!["test"],
        ProjectCheckKind::Lint => vec!["clippy", "--all-targets"],
        ProjectCheckKind::Build => vec!["build"],
        ProjectCheckKind::FormatCheck => vec!["fmt", "--all", "--", "--check"],
    }
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    ProjectCheck {
        label: format!("Running cargo {}", args.join(" ")),
        command: "cargo".to_owned(),
        args,
    }
}

fn select_flutter_check(kind: ProjectCheckKind) -> Result<ProjectCheck, String> {
    if matches!(kind, ProjectCheckKind::Build) {
        return Err(
            "Flutter build target is ambiguous. Ask for the exact target, then use run_project_command."
                .to_owned(),
        );
    }
    if matches!(kind, ProjectCheckKind::Test) {
        return Ok(ProjectCheck {
            command: "flutter".to_owned(),
            args: vec!["test".to_owned()],
            label: "Running flutter test".to_owned(),
        });
    }
    if matches!(kind, ProjectCheckKind::FormatCheck) {
        return Ok(ProjectCheck {
            command: "dart".to_owned(),
            args: vec![
                "format".to_owned(),
                "--output=none".to_owned(),
                "--set-exit-if-changed".to_owned(),
                ".".to_owned(),
            ],
            label: "Running Dart format check".to_owned(),
        });
    }
    Ok(ProjectCheck {
        command: "flutter".to_owned(),
        args: vec!["analyze".to_owned()],
        label: "Running flutter analyze".to_owned(),
    })
}

async fn detect_node_package_manager(root: &Path) -> String {
    for (lockfile, manager) in [("pnpm-lock.yaml", "pnpm"), ("yarn.lock", "yarn")] {
        if fs::metadata(root.join(lockfile))
            .await
            .is_ok_and(|metadata| metadata.is_file())
        {
            return manager.to_owned();
        }
    }
    "npm".to_owned()
}

fn node_candidates(kind: ProjectCheckKind) -> Vec<&'static str> {
    match kind {
        ProjectCheckKind::Auto => vec!["check", "typecheck", "test", "lint", "build"],
        ProjectCheckKind::Test => vec!["test"],
        ProjectCheckKind::Check => vec!["check", "typecheck", "test"],
        ProjectCheckKind::Typecheck => vec!["typecheck", "type-check"],
        ProjectCheckKind::Lint => vec!["lint"],
        ProjectCheckKind::Build => vec!["build"],
        ProjectCheckKind::FormatCheck => vec!["format:check", "fmt:check", "prettier:check"],
    }
}

impl ProjectCheckKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Test => "test",
            Self::Check => "check",
            Self::Typecheck => "typecheck",
            Self::Lint => "lint",
            Self::Build => "build",
            Self::FormatCheck => "format-check",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn detects_rust_and_selects_safe_checks() {
        let temp = tempdir().expect("temporary directory");
        fs::write(temp.path().join("Cargo.toml"), "[package]\nname='demo'\n").expect("manifest");
        assert_eq!(detect_project_type(temp.path()).await, "rust");
        assert_eq!(
            select_project_check(temp.path(), ProjectCheckKind::FormatCheck)
                .await
                .expect("format check")
                .args,
            ["fmt", "--all", "--", "--check"]
        );
    }

    #[tokio::test]
    async fn selects_node_script_and_package_manager() {
        let temp = tempdir().expect("temporary directory");
        fs::write(
            temp.path().join("package.json"),
            r#"{"scripts":{"typecheck":"tsc --noEmit"}}"#,
        )
        .expect("package manifest");
        fs::write(temp.path().join("pnpm-lock.yaml"), "lockfileVersion: 9\n").expect("lockfile");
        let check = select_project_check(temp.path(), ProjectCheckKind::Typecheck)
            .await
            .expect("node check");
        assert_eq!(check.command, "pnpm");
        assert_eq!(check.args, ["run", "typecheck"]);
    }
}
