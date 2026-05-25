//! reachpy-config
//! 
//! The centralized config system for ReachPy. Every component — the hot reloader,
//! the CLI, the bundler, the node runner — goes through here. Nobody parses 
//! reach.toml themselves.
//!
//! Responsibilities:
//!   - Parse reach.toml into typed structs
//!   - Validate with human-readable errors
//!   - Resolve relative paths to absolute
//!   - Apply defaults for optional fields
//!   - Locate project root by walking up from cwd

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

// ─── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("No reach.toml found. Are you inside a ReachPy workspace?\n  Run `reach create <name>` to create one.")]
    NotFound,

    #[error("Could not read reach.toml: {0}")]
    ReadError(#[from] std::io::Error),

    #[error("reach.toml is invalid:\n  {0}\n\n  Run `reach doctor` to diagnose config issues.")]
    ParseError(String),

    #[error("reach.toml validation failed:\n{0}")]
    ValidationError(String),
}

// ─── Raw deserialization structs (what toml gives us) ─────────────────────────
// These are permissive — all fields optional so we can give good validation errors

#[derive(Debug, Deserialize)]
struct RawConfig {
    project: Option<RawProject>,
    robot: Option<RawRobot>,
    nodes: Option<HashMap<String, String>>,
    launch: Option<HashMap<String, Vec<String>>>,
    dependencies: Option<HashMap<String, toml::Value>>,
    dev: Option<RawDev>,
    ros: Option<RawRos>,
}

#[derive(Debug, Deserialize)]
struct RawProject {
    name: Option<String>,
    version: Option<String>,
    python: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawRobot {
    platform: Option<String>,
    transport: Option<String>,
    domain_id: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct RawDev {
    hot_reload: Option<bool>,
    hot_reload_ignore: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct RawRos {
    // Escape hatch section — for the 20% of cases reach.toml can't cover cleanly
    // Topic remapping, namespacing, per-node ROS args
    remappings: Option<HashMap<String, String>>,       // topic remaps: {"/old": "/new"}
    namespace: Option<String>,                          // global namespace prefix
    node_args: Option<HashMap<String, Vec<String>>>,   // per-node extra ros args
    parameters: Option<HashMap<String, toml::Value>>,  // global ros parameters
}

// ─── Resolved config structs (what the rest of ReachPy uses) ─────────────────

#[derive(Debug, Clone)]
pub struct ReachConfig {
    /// Absolute path to the project root (where reach.toml lives)
    pub root: PathBuf,

    pub project: ProjectConfig,
    pub robot: RobotConfig,
    pub nodes: HashMap<String, NodeConfig>,
    pub launch: LaunchConfig,
    pub dependencies: Vec<Dependency>,
    pub dev: DevConfig,
    pub ros: RosConfig,
}

#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub name: String,
    pub version: String,
    pub python: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RobotConfig {
    pub platform: String,
    pub transport: Transport,
    pub domain_id: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Transport {
    Ros2,
    // Ros1 in future
}

impl std::fmt::Display for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Transport::Ros2 => write!(f, "ros2"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Node name as declared in [nodes]
    pub name: String,
    /// Absolute path to the Python script
    pub script: PathBuf,
    /// Relative path as written in reach.toml (for display)
    pub script_rel: String,
}

#[derive(Debug, Clone)]
pub struct LaunchConfig {
    /// Named launch profiles. "default" is used when no profile specified.
    /// Maps profile name -> list of node names
    pub profiles: HashMap<String, Vec<String>>,
}

impl LaunchConfig {
    /// Get the node names for a launch profile.
    /// Falls back to all nodes if "default" profile not defined.
    pub fn resolve_profile<'a>(
        &'a self,
        profile: &str,
        all_nodes: &'a HashMap<String, NodeConfig>,
    ) -> Option<Vec<&'a str>> {
        if let Some(names) = self.profiles.get(profile) {
            Some(names.iter().map(|s| s.as_str()).collect())
        } else if profile == "default" {
            // No default profile defined — run everything
            Some(all_nodes.keys().map(|s| s.as_str()).collect())
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DevConfig {
    pub hot_reload: bool,
    pub hot_reload_ignore: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RosConfig {
    /// Topic remappings — replaces <remap> in XML launch files
    pub remappings: HashMap<String, String>,
    /// Global namespace — replaces <group ns="..."> in XML launch files  
    pub namespace: Option<String>,
    /// Per-node extra ROS args — escape hatch for advanced users
    pub node_args: HashMap<String, Vec<String>>,
    /// Global ROS parameters
    pub parameters: HashMap<String, String>,
}

// ─── Loader ───────────────────────────────────────────────────────────────────

impl ReachConfig {
    /// Load from a specific path
    pub fn from_path(config_path: &Path) -> Result<Self, ConfigError> {
        let root = config_path
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();

        let content = std::fs::read_to_string(config_path)?;
        let raw: RawConfig = toml::from_str(&content)
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;

        Self::resolve(raw, root)
    }

    /// Locate reach.toml by walking up from cwd, then load
    pub fn load() -> Result<Self, ConfigError> {
        let config_path = find_config_file()?;
        Self::from_path(&config_path)
    }

    /// Resolve raw deserialized config into validated, typed config
    fn resolve(raw: RawConfig, root: PathBuf) -> Result<Self, ConfigError> {
        let mut errors: Vec<String> = Vec::new();

        // ── [project] ────────────────────────────────────────────────────────
        let project = match raw.project {
            None => {
                errors.push("  Missing required section [project]".to_string());
                None
            }
            Some(p) => {
                let name = require_field(&p.name, "[project] name", &mut errors);
                let version = p.version.unwrap_or_else(|| "0.1.0".to_string());
                let python = p.python.clone().unwrap_or_else(|| "3.11".to_string());

                // Validate python version format
                if let Some(ref py) = p.python {
                    if !py.starts_with("3.") {
                        errors.push(format!(
                            "  [project] python must be a Python 3.x version, got \"{}\"", py
                        ));
                    }
                }

                name.map(|n| ProjectConfig {
                    name: n,
                    version,
                    python,
                    description: p.description,
                })
            }
        };

        // ── [robot] ──────────────────────────────────────────────────────────
        let robot = match raw.robot {
            None => {
                // Robot section is optional — defaults to ros2
                Some(RobotConfig {
                    platform: "generic".to_string(),
                    transport: Transport::Ros2,
                    domain_id: 0,
                })
            }
            Some(r) => {
                let transport = match r.transport.as_deref() {
                    None | Some("ros2") => Transport::Ros2,
                    Some(other) => {
                        errors.push(format!(
                            "  [robot] transport \"{}\" is not supported. Use \"ros2\".", other
                        ));
                        Transport::Ros2
                    }
                };
                Some(RobotConfig {
                    platform: r.platform.unwrap_or_else(|| "generic".to_string()),
                    transport,
                    domain_id: r.domain_id.unwrap_or(0),
                })
            }
        };

        // ── [nodes] ──────────────────────────────────────────────────────────
        let mut nodes: HashMap<String, NodeConfig> = HashMap::new();
        if let Some(raw_nodes) = raw.nodes {
            if raw_nodes.is_empty() {
                errors.push("  [nodes] is empty. Add at least one node.".to_string());
            }
            for (name, script_rel) in &raw_nodes {
                let script = root.join(script_rel);
                if !script.exists() {
                    errors.push(format!(
                        "  [nodes] \"{}\": script not found at \"{}\"",
                        name, script_rel
                    ));
                } else {
                    nodes.insert(name.clone(), NodeConfig {
                        name: name.clone(),
                        script,
                        script_rel: script_rel.clone(),
                    });
                }
            }
        } else {
            errors.push("  Missing required section [nodes]".to_string());
        }

        // ── [launch] ─────────────────────────────────────────────────────────
        let launch = {
            let mut profiles: HashMap<String, Vec<String>> = HashMap::new();
            if let Some(raw_launch) = raw.launch {
                for (profile, node_names) in raw_launch {
                    // Validate every referenced node exists
                    for node_name in &node_names {
                        if !nodes.contains_key(node_name) {
                            errors.push(format!(
                                "  [launch] profile \"{}\" references unknown node \"{}\"",
                                profile, node_name
                            ));
                        }
                    }
                    profiles.insert(profile, node_names);
                }
            }
            LaunchConfig { profiles }
        };

        // ── [dependencies] ───────────────────────────────────────────────────
        let dependencies = raw.dependencies
            .unwrap_or_default()
            .into_iter()
            .map(|(name, val)| {
                let version = match val {
                    toml::Value::String(s) => Some(s),
                    _ => None,
                };
                Dependency { name, version }
            })
            .collect();

        // ── [dev] ────────────────────────────────────────────────────────────
        let dev = {
            let d = raw.dev.unwrap_or(RawDev {
                hot_reload: None,
                hot_reload_ignore: None,
            });
            DevConfig {
                hot_reload: d.hot_reload.unwrap_or(true),
                hot_reload_ignore: d.hot_reload_ignore.unwrap_or_default(),
            }
        };

        // ── [ros] ────────────────────────────────────────────────────────────
        let ros = {
            let r = raw.ros.unwrap_or(RawRos {
                remappings: None,
                namespace: None,
                node_args: None,
                parameters: None,
            });
            RosConfig {
                remappings: r.remappings.unwrap_or_default(),
                namespace: r.namespace,
                node_args: r.node_args.unwrap_or_default(),
                parameters: r.parameters
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|(k, v)| {
                        if let toml::Value::String(s) = v { Some((k, s)) } else { None }
                    })
                    .collect(),
            }
        };

        // ── Bail if any validation errors ────────────────────────────────────
        if !errors.is_empty() {
            return Err(ConfigError::ValidationError(errors.join("\n")));
        }

        Ok(ReachConfig {
            root,
            project: project.unwrap(),
            robot: robot.unwrap(),
            nodes,
            launch,
            dependencies,
            dev,
            ros,
        })
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Walk up from cwd to find reach.toml
fn find_config_file() -> Result<PathBuf, ConfigError> {
    let mut dir = std::env::current_dir()
        .map_err(|e| ConfigError::ReadError(e))?;
    loop {
        let candidate = dir.join("reach.toml");
        if candidate.exists() {
            return Ok(candidate);
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return Err(ConfigError::NotFound),
        }
    }
}

fn require_field(val: &Option<String>, field: &str, errors: &mut Vec<String>) -> Option<String> {
    match val {
        Some(v) if !v.trim().is_empty() => Some(v.clone()),
        _ => {
            errors.push(format!("  Missing required field: {}", field));
            None
        }
    }
}

// ─── Display ──────────────────────────────────────────────────────────────────

impl std::fmt::Display for ReachConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        writeln!(f, "Project:     {} v{}", self.project.name, self.project.version)?;
        writeln!(f, "Python:      {}", self.project.python)?;
        writeln!(f, "Platform:    {}", self.robot.platform)?;
        writeln!(f, "Transport:   {}", self.robot.transport)?;
        writeln!(f, "Domain ID:   {}", self.robot.domain_id)?;
        writeln!(f, "Nodes:       {}", self.nodes.len())?;
        for (name, node) in &self.nodes {
            writeln!(f, "  - {} → {}", name, node.script_rel)?;
        }
        if !self.launch.profiles.is_empty() {
            writeln!(f, "Launch profiles:")?;
            for (profile, nodes) in &self.launch.profiles {
                writeln!(f, "  - {} → [{}]", profile, nodes.join(", "))?;
            }
        }
        writeln!(f, "Hot reload:  {}", self.dev.hot_reload)?;
        if !self.ros.remappings.is_empty() {
            writeln!(f, "Remappings:")?;
            for (from, to) in &self.ros.remappings {
                writeln!(f, "  {} → {}", from, to)?;
            }
        }
        Ok(())
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_config(dir: &TempDir, content: &str) -> PathBuf {
        // Create a dummy node script so path validation passes
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/detector.py"), "# node").unwrap();
        let path = dir.path().join("reach.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "{}", content).unwrap();
        path
    }

    #[test]
    fn test_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(&dir, r#"
[project]
name = "my-robot"
version = "0.1.0"
python = "3.11"

[robot]
platform = "ur5"
transport = "ros2"
domain_id = 0

[nodes]
detector = "src/detector.py"

[launch]
default = ["detector"]

[dev]
hot_reload = true
"#);
        let config = ReachConfig::from_path(&path).unwrap();
        assert_eq!(config.project.name, "my-robot");
        assert_eq!(config.nodes.len(), 1);
        assert!(config.nodes.contains_key("detector"));
        assert_eq!(config.dev.hot_reload, true);
    }

    #[test]
    fn test_missing_project_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(&dir, r#"
[project]
version = "0.1.0"

[nodes]
detector = "src/detector.py"
"#);
        let err = ReachConfig::from_path(&path).unwrap_err();
        assert!(err.to_string().contains("project] name"));
    }

    #[test]
    fn test_unknown_launch_node() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(&dir, r#"
[project]
name = "my-robot"

[nodes]
detector = "src/detector.py"

[launch]
default = ["detector", "ghost-node"]
"#);
        let err = ReachConfig::from_path(&path).unwrap_err();
        assert!(err.to_string().contains("ghost-node"));
    }

    #[test]
    fn test_defaults_applied() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(&dir, r#"
[project]
name = "minimal"

[nodes]
detector = "src/detector.py"
"#);
        let config = ReachConfig::from_path(&path).unwrap();
        assert_eq!(config.project.version, "0.1.0");
        assert_eq!(config.project.python, "3.11");
        assert_eq!(config.robot.platform, "generic");
        assert_eq!(config.robot.domain_id, 0);
        assert_eq!(config.dev.hot_reload, true);
    }

    #[test]
    fn test_remappings_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(&dir, r#"
[project]
name = "remap-test"

[nodes]
detector = "src/detector.py"

[ros]
namespace = "/robot1"

[ros.remappings]
"/camera/raw" = "/camera/compressed"
"#);
        let config = ReachConfig::from_path(&path).unwrap();
        assert_eq!(config.ros.namespace, Some("/robot1".to_string()));
        assert_eq!(
            config.ros.remappings.get("/camera/raw"),
            Some(&"/camera/compressed".to_string())
        );
    }
}