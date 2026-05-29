//! reachpy-config
//!
//! Centralized config system for ReachPy. Every component goes through here.
//! Parses, validates, and resolves reach.toml into typed structs.

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

    #[error("reach.toml is invalid:\n  {0}")]
    ParseError(String),

    #[error("reach.toml validation failed:\n{0}")]
    ValidationError(String),
}

// ─── Raw structs (permissive — all optional for good error messages) ──────────

#[derive(Debug, Deserialize)]
struct RawConfig {
    project:      Option<RawProject>,
    robot:        Option<RawRobot>,
    nodes:        Option<HashMap<String, String>>,
    launch:       Option<HashMap<String, RawLaunchPipeline>>,
    dependencies: Option<HashMap<String, toml::Value>>,
    dev:          Option<RawDev>,
    ros:          Option<RawRos>,
}

#[derive(Debug, Deserialize)]
struct RawProject {
    name:        Option<String>,
    version:     Option<String>,
    python:      Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawRobot {
    platform:  Option<String>,
    transport: Option<String>,
    domain_id: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct RawDev {
    hot_reload:        Option<bool>,
    hot_reload_ignore: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct RawRos {
    remappings: Option<HashMap<String, String>>,
    namespace:  Option<String>,
    node_args:  Option<HashMap<String, Vec<String>>>,
    parameters: Option<HashMap<String, toml::Value>>,
}

/// A launch pipeline — [launch.prod], [launch.default], etc.
#[derive(Debug, Deserialize)]
struct RawLaunchPipeline {
    nodes: Option<Vec<RawLaunchNode>>,
}

/// A single node entry inside a launch pipeline
#[derive(Debug, Deserialize)]
struct RawLaunchNode {
    name:       Option<String>,
    delay:      Option<f64>,
    depends_on: Option<String>,
    wait_ready: Option<bool>,
    params:     Option<HashMap<String, String>>,
}

// ─── Resolved structs (typed, validated, ready to use) ───────────────────────

#[derive(Debug, Clone)]
pub struct ReachConfig {
    pub root:         PathBuf,
    pub project:      ProjectConfig,
    pub robot:        RobotConfig,
    pub nodes:        HashMap<String, NodeConfig>,
    pub launch:       HashMap<String, LaunchPipeline>,
    pub dependencies: Vec<Dependency>,
    pub dev:          DevConfig,
    pub ros:          RosConfig,
}

#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub name:        String,
    pub version:     String,
    pub python:      String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RobotConfig {
    pub platform:  String,
    pub transport: Transport,
    pub domain_id: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Transport {
    Ros2,
}

impl std::fmt::Display for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self { Transport::Ros2 => write!(f, "ros2") }
    }
}

#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub name:       String,
    pub script:     PathBuf,
    pub script_rel: String,
}

/// A resolved launch pipeline — [launch.prod], [launch.default], etc.
#[derive(Debug, Clone)]
pub struct LaunchPipeline {
    pub name:  String,
    pub nodes: Vec<LaunchNode>,
}

/// A single node step inside a pipeline
#[derive(Debug, Clone)]
pub struct LaunchNode {
    /// Node name — must exist in [nodes]
    pub name:       String,
    /// Seconds to wait before starting this node
    pub delay:      f64,
    /// Wait for this other node to be running first
    pub depends_on: Option<String>,
    /// Block until this node is visible on the ROS2 graph before proceeding
    pub wait_ready: bool,
    /// ROS2 parameters to pass — become --ros-args -p key:=value
    pub params:     HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name:    String,
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DevConfig {
    pub hot_reload:        bool,
    pub hot_reload_ignore: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RosConfig {
    pub remappings: HashMap<String, String>,
    pub namespace:  Option<String>,
    pub node_args:  HashMap<String, Vec<String>>,
    pub parameters: HashMap<String, String>,
}

// ─── Loader ───────────────────────────────────────────────────────────────────

impl ReachConfig {
    pub fn load() -> Result<Self, ConfigError> {
        let path = find_config_file()?;
        Self::from_path(&path)
    }

    pub fn from_path(config_path: &Path) -> Result<Self, ConfigError> {
        let root = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let content = std::fs::read_to_string(config_path)?;
        let raw: RawConfig = toml::from_str(&content)
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;
        Self::resolve(raw, root)
    }

    fn resolve(raw: RawConfig, root: PathBuf) -> Result<Self, ConfigError> {
        let mut errors: Vec<String> = Vec::new();

        // ── [project] ────────────────────────────────────────────────────────
        let project = match raw.project {
            None => { errors.push("  Missing required section [project]".into()); None }
            Some(p) => {
                let name = require_field(&p.name, "[project] name", &mut errors);
                let version = p.version.unwrap_or_else(|| "0.1.0".into());
                let python  = p.python.clone().unwrap_or_else(|| "3.11".into());
                if let Some(ref py) = p.python {
                    if !py.starts_with("3.") {
                        errors.push(format!("  [project] python must be 3.x, got \"{}\"", py));
                    }
                }
                name.map(|n| ProjectConfig { name: n, version, python, description: p.description })
            }
        };

        // ── [robot] ──────────────────────────────────────────────────────────
        let robot = match raw.robot {
            None => Some(RobotConfig { platform: "generic".into(), transport: Transport::Ros2, domain_id: 0 }),
            Some(r) => {
                let transport = match r.transport.as_deref() {
                    None | Some("ros2") => Transport::Ros2,
                    Some(other) => {
                        errors.push(format!("  [robot] transport \"{}\" not supported. Use \"ros2\".", other));
                        Transport::Ros2
                    }
                };
                Some(RobotConfig {
                    platform:  r.platform.unwrap_or_else(|| "generic".into()),
                    transport,
                    domain_id: r.domain_id.unwrap_or(0),
                })
            }
        };

        // ── [nodes] ──────────────────────────────────────────────────────────
        let mut nodes: HashMap<String, NodeConfig> = HashMap::new();
        match raw.nodes {
            None => errors.push("  Missing required section [nodes]".into()),
            Some(raw_nodes) => {
                if raw_nodes.is_empty() {
                    errors.push("  [nodes] is empty. Add at least one node.".into());
                }
                for (name, script_rel) in &raw_nodes {
                    let script = root.join(script_rel);
                    if !script.exists() {
                        errors.push(format!("  [nodes] \"{}\": script not found at \"{}\"", name, script_rel));
                    } else {
                        nodes.insert(name.clone(), NodeConfig {
                            name: name.clone(),
                            script,
                            script_rel: script_rel.clone(),
                        });
                    }
                }
            }
        }

        // ── [launch.*] ───────────────────────────────────────────────────────
        // Each [launch.prod], [launch.default] etc becomes a LaunchPipeline
        let mut launch: HashMap<String, LaunchPipeline> = HashMap::new();
        if let Some(raw_launch) = raw.launch {
            for (pipeline_name, raw_pipeline) in raw_launch {
                let mut pipeline_nodes: Vec<LaunchNode> = Vec::new();

                let raw_nodes = raw_pipeline.nodes.unwrap_or_default();
                if raw_nodes.is_empty() {
                    errors.push(format!(
                        "  [launch.{}] has no nodes defined.", pipeline_name
                    ));
                    continue;
                }

                for (i, raw_node) in raw_nodes.iter().enumerate() {
                    // name is required per node entry
                    let node_name = match &raw_node.name {
                        Some(n) => n.clone(),
                        None => {
                            errors.push(format!(
                                "  [launch.{}] node at index {} is missing \"name\"",
                                pipeline_name, i
                            ));
                            continue;
                        }
                    };

                    // node must exist in [nodes]
                    if !nodes.contains_key(&node_name) {
                        errors.push(format!(
                            "  [launch.{}] references unknown node \"{}\"",
                            pipeline_name, node_name
                        ));
                        continue;
                    }

                    // depends_on must also be a known node
                    if let Some(ref dep) = raw_node.depends_on {
                        if !nodes.contains_key(dep) {
                            errors.push(format!(
                                "  [launch.{}] node \"{}\" depends_on unknown node \"{}\"",
                                pipeline_name, node_name, dep
                            ));
                        }
                    }

                    // delay must be non-negative
                    let delay = raw_node.delay.unwrap_or(0.0);
                    if delay < 0.0 {
                        errors.push(format!(
                            "  [launch.{}] node \"{}\" delay must be >= 0, got {}",
                            pipeline_name, node_name, delay
                        ));
                    }

                    pipeline_nodes.push(LaunchNode {
                        name:       node_name,
                        delay,
                        depends_on: raw_node.depends_on.clone(),
                        wait_ready: raw_node.wait_ready.unwrap_or(false),
                        params:     raw_node.params.clone().unwrap_or_default(),
                    });
                }

                launch.insert(pipeline_name.clone(), LaunchPipeline {
                    name:  pipeline_name,
                    nodes: pipeline_nodes,
                });
            }
        }

        // ── [dependencies] ───────────────────────────────────────────────────
        let dependencies = raw.dependencies.unwrap_or_default()
            .into_iter()
            .map(|(name, val)| Dependency {
                name,
                version: if let toml::Value::String(s) = val { Some(s) } else { None },
            })
            .collect();

        // ── [dev] ────────────────────────────────────────────────────────────
        let dev = {
            let d = raw.dev.unwrap_or(RawDev { hot_reload: None, hot_reload_ignore: None });
            DevConfig {
                hot_reload:        d.hot_reload.unwrap_or(true),
                hot_reload_ignore: d.hot_reload_ignore.unwrap_or_default(),
            }
        };

        // ── [ros] ────────────────────────────────────────────────────────────
        let ros = {
            let r = raw.ros.unwrap_or(RawRos {
                remappings: None, namespace: None, node_args: None, parameters: None,
            });
            RosConfig {
                remappings: r.remappings.unwrap_or_default(),
                namespace:  r.namespace,
                node_args:  r.node_args.unwrap_or_default(),
                parameters: r.parameters.unwrap_or_default()
                    .into_iter()
                    .filter_map(|(k, v)| if let toml::Value::String(s) = v { Some((k, s)) } else { None })
                    .collect(),
            }
        };

        // ── Bail on errors ───────────────────────────────────────────────────
        if !errors.is_empty() {
            return Err(ConfigError::ValidationError(errors.join("\n")));
        }

        Ok(ReachConfig { root, project: project.unwrap(), robot: robot.unwrap(), nodes, launch, dependencies, dev, ros })
    }
}

// ─── Display ─────────────────────────────────────────────────────────────────

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
        if !self.launch.is_empty() {
            writeln!(f, "Pipelines:")?;
            for (name, pipeline) in &self.launch {
                let node_names: Vec<_> = pipeline.nodes.iter().map(|n| n.name.as_str()).collect();
                writeln!(f, "  - {} → [{}]", name, node_names.join(", "))?;
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

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn find_config_file() -> Result<PathBuf, ConfigError> {
    let mut dir = std::env::current_dir().map_err(ConfigError::ReadError)?;
    loop {
        let candidate = dir.join("reach.toml");
        if candidate.exists() { return Ok(candidate); }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return Err(ConfigError::NotFound),
        }
    }
}

fn require_field(val: &Option<String>, field: &str, errors: &mut Vec<String>) -> Option<String> {
    match val {
        Some(v) if !v.trim().is_empty() => Some(v.clone()),
        _ => { errors.push(format!("  Missing required field: {}", field)); None }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn setup(dir: &TempDir, content: &str) -> PathBuf {
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/detector.py"), "# node").unwrap();
        std::fs::write(dir.path().join("src/controller.py"), "# node").unwrap();
        let path = dir.path().join("reach.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "{}", content).unwrap();
        path
    }

    #[test]
    fn test_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup(&dir, r#"
[project]
name = "my-robot"
version = "0.1.0"
python = "3.11"

[nodes]
detector = "src/detector.py"

[launch.default]
nodes = [
    { name = "detector" }
]
"#);
        let config = ReachConfig::from_path(&path).unwrap();
        assert_eq!(config.project.name, "my-robot");
        assert!(config.launch.contains_key("default"));
        assert_eq!(config.launch["default"].nodes.len(), 1);
    }

    #[test]
    fn test_pipeline_with_params_and_delay() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup(&dir, r#"
[project]
name = "my-robot"

[nodes]
detector   = "src/detector.py"
controller = "src/controller.py"

[launch.prod]
nodes = [
    { name = "detector",   delay = 0.0, wait_ready = true, params = { model = "yolo.pt" } },
    { name = "controller", delay = 2.0, depends_on = "detector" },
]
"#);
        let config = ReachConfig::from_path(&path).unwrap();
        let prod = &config.launch["prod"];
        assert_eq!(prod.nodes.len(), 2);
        assert_eq!(prod.nodes[0].delay, 0.0);
        assert!(prod.nodes[0].wait_ready);
        assert_eq!(prod.nodes[0].params.get("model"), Some(&"yolo.pt".to_string()));
        assert_eq!(prod.nodes[1].delay, 2.0);
        assert_eq!(prod.nodes[1].depends_on, Some("detector".to_string()));
    }

    #[test]
    fn test_multiple_pipelines() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup(&dir, r#"
[project]
name = "my-robot"

[nodes]
detector   = "src/detector.py"
controller = "src/controller.py"

[launch.default]
nodes = [{ name = "detector" }]

[launch.full]
nodes = [
    { name = "detector" },
    { name = "controller", delay = 1.0 },
]
"#);
        let config = ReachConfig::from_path(&path).unwrap();
        assert!(config.launch.contains_key("default"));
        assert!(config.launch.contains_key("full"));
        assert_eq!(config.launch["full"].nodes.len(), 2);
    }

    #[test]
    fn test_unknown_node_in_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup(&dir, r#"
[project]
name = "my-robot"

[nodes]
detector = "src/detector.py"

[launch.prod]
nodes = [{ name = "ghost" }]
"#);
        let err = ReachConfig::from_path(&path).unwrap_err();
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn test_missing_project_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup(&dir, r#"
[project]
version = "0.1.0"

[nodes]
detector = "src/detector.py"
"#);
        let err = ReachConfig::from_path(&path).unwrap_err();
        assert!(err.to_string().contains("project] name"));
    }

    #[test]
    fn test_defaults_applied() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup(&dir, r#"
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
}
