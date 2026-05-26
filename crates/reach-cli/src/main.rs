use colored::Colorize;
use reachpy_config::{ConfigError, ReachConfig};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ─── Embedded launcher.py ─────────────────────────────────────────────────────
// Baked into the binary at compile time. No external file needed.
const LAUNCHER_PY: &str = include_str!("../python/launcher.py");

// ─── Banner ───────────────────────────────────────────────────────────────────

fn print_banner() {
    println!();
    println!("{}", "  ██████╗ ███████╗ █████╗  ██████╗██╗  ██╗".bright_blue());
    println!("{}", "  ██╔══██╗██╔════╝██╔══██╗██╔════╝██║  ██║".bright_blue());
    println!("{}", "  ██████╔╝█████╗  ███████║██║     ███████║".bright_blue());
    println!("{}", "  ██╔══██╗██╔══╝  ██╔══██║██║     ██╔══██║".bright_blue());
    println!("{}", "  ██║  ██║███████╗██║  ██║╚██████╗██║  ██║".bright_blue());
    println!("{}", "  ╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝ ╚═════╝╚═╝  ╚═╝".bright_blue());
    println!();
}

fn print_help() {
    print_banner();
    println!("  {} {}", "reach".white().bold(), "— Make your code reach the robot.".dimmed());
    println!();
    println!("  {}", "COMMANDS".dimmed());
    println!("  {}  {}  {}", "reach create".cyan().bold(), "<name>".white(),    "Create a new ReachPy workspace".dimmed());
    println!("  {}          {}", "reach config".cyan().bold(),                  "Show resolved config for current workspace".dimmed());
    println!("  {}  {}  {}", "reach run".cyan().bold(), "[profile]".white(),    "Run a launch profile (default: 'default')".dimmed());
    println!("  {}    {}",    "reach dev".cyan().bold(),                        "Start hot-reload dev server  (coming soon)".dimmed());
    println!("  {}  {}",      "reach build".cyan().bold(),                      "Bundle for production         (coming soon)".dimmed());
    println!("  {}  {}",      "reach doctor".cyan().bold(),                     "Diagnose workspace issues     (coming soon)".dimmed());
    println!();
}

// ─── reach create ─────────────────────────────────────────────────────────────

fn cmd_create(name: &str) {
    let workspace = std::path::Path::new(name);

    if workspace.exists() {
        eprintln!("  {} Directory \"{}\" already exists.", "✗".red().bold(), name);
        std::process::exit(1);
    }

    println!();
    println!("  {} Creating ReachPy workspace {}...", "→".bright_blue().bold(), name.white().bold());

    let dirs = ["src", "config", "models", "launch"];
    for dir in &dirs {
        std::fs::create_dir_all(workspace.join(dir))
            .unwrap_or_else(|e| fatal(&format!("Failed to create {}: {}", dir, e)));
    }

    let toml = format!(
r#"# ─────────────────────────────────────────────
#  reach.toml — ReachPy workspace configuration
#  This single file replaces:
#    - CMakeLists.txt
#    - package.xml
#    - setup.py
#    - XML launch files
# ─────────────────────────────────────────────

[project]
name = "{name}"
version = "0.1.0"
python = "3.11"
description = "A ReachPy robotics workspace"

[robot]
platform = "generic"
transport = "ros2"
domain_id = 0

[nodes]
example = "src/example.py"

[launch]
default = ["example"]

[dev]
hot_reload = true
hot_reload_ignore = ["config/", "models/"]
"#,
        name = name
    );

    std::fs::write(workspace.join("reach.toml"), toml)
        .unwrap_or_else(|e| fatal(&format!("Failed to write reach.toml: {}", e)));

    let example_node = r#"# ReachPy example node
# This runs as a real ROS2 node via `reach run`
import time

print("[example] Node started — make your code reach the robot.")

i = 0
while True:
    print(f"[example] tick {i}")
    i += 1
    time.sleep(1)
"#;

    std::fs::write(workspace.join("src/example.py"), example_node)
        .unwrap_or_else(|e| fatal(&format!("Failed to write example node: {}", e)));

    let gitignore = "# ReachPy\n.reach/\ndist/\n__pycache__/\n*.pyc\n*.pyo\nmodels/\n.env\n";
    std::fs::write(workspace.join(".gitignore"), gitignore)
        .unwrap_or_else(|e| fatal(&format!("Failed to write .gitignore: {}", e)));

    println!();
    println!("  {} {}", "✓".green().bold(), format!("Created workspace \"{}\"", name).white().bold());
    println!();
    println!("  {}", "Structure:".dimmed());
    println!("  {}  {}", "├── reach.toml".cyan(),      "← your entire config lives here".dimmed());
    println!("  {}  {}", "├── src/".white(),            "← node scripts".dimmed());
    println!("  {}  {}", "│   └── example.py".white(), "← example node".dimmed());
    println!("  {}  {}", "├── config/".white(),         "← robot config (not hot-reloaded)".dimmed());
    println!("  {}  {}", "└── models/".white(),         "← ML models (not hot-reloaded)".dimmed());
    println!();
    println!("  {}", "Next steps:".dimmed());
    println!("  {}  {}", "cd".cyan(), name.white().bold());
    println!("  {}", "reach run".cyan().bold());
    println!();
    println!("  {} Make your code reach the robot.", "→".bright_blue().bold());
    println!();
}

// ─── reach config ─────────────────────────────────────────────────────────────

fn cmd_config() {
    match ReachConfig::load() {
        Ok(config) => {
            println!();
            println!("  {} {}", "✓".green().bold(), "reach.toml resolved successfully".white().bold());
            println!();
            println!("  {}", "─".repeat(44).dimmed());
            for line in config.to_string().lines() {
                println!("  {}", line.dimmed());
            }
            println!("  {}", "─".repeat(44).dimmed());
            println!();
        }
        Err(ConfigError::NotFound) => {
            eprintln!();
            eprintln!("  {} {}", "✗".red().bold(), "No reach.toml found.".red());
            eprintln!("  Run {} to create a workspace.", "reach create <name>".cyan());
            eprintln!();
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!();
            eprintln!("  {} {}", "✗".red().bold(), "Config error:".red().bold());
            eprintln!();
            for line in e.to_string().lines() {
                eprintln!("  {}", line.red());
            }
            eprintln!();
            std::process::exit(1);
        }
    }
}

// ─── reach run ────────────────────────────────────────────────────────────────

fn cmd_run(profile: &str) {
    // Load and validate config
    let config = match ReachConfig::load() {
        Ok(c) => c,
        Err(ConfigError::NotFound) => {
            eprintln!("  {} No reach.toml found. Are you inside a ReachPy workspace?", "✗".red().bold());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("  {} {}", "✗".red().bold(), e.to_string().red());
            std::process::exit(1);
        }
    };

    // Resolve which nodes to run
    let node_names = match config.launch.resolve_profile(profile, &config.nodes) {
        Some(names) => names,
        None => {
            eprintln!(
                "  {} Launch profile \"{}\" not found in reach.toml.",
                "✗".red().bold(), profile
            );
            eprintln!("  Available profiles: {}",
                config.launch.profiles.keys()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            std::process::exit(1);
        }
    };

    print_banner();
    println!(
        "  {} {}  {}",
        "reach run".white().bold(),
        format!("profile: {}", profile).dimmed(),
        format!("({} node{})", node_names.len(), if node_names.len() == 1 { "" } else { "s" }).dimmed()
    );
    println!();

    // Write embedded launcher.py to a temp file
    let launcher_path = write_launcher().unwrap_or_else(|e| {
        fatal(&format!("Failed to write launcher: {}", e))
    });

    // Detect ROS2 installation
    let ros_setup = find_ros2_setup();
    if ros_setup.is_none() {
        eprintln!(
            "  {} ROS2 not found. Make sure ROS2 is installed and sourced.",
            "✗".red().bold()
        );
        eprintln!("  Try: {}", "source /opt/ros/humble/setup.bash".cyan());
        std::process::exit(1);
    }
    let ros_setup = ros_setup.unwrap();
    println!("  {} {}", "ROS2:".dimmed(), ros_setup.display().to_string().white());
    println!("  {} {}", "domain_id:".dimmed(), config.robot.domain_id.to_string().white());
    println!();

    // Spawn all nodes
    let processes: Arc<Mutex<Vec<(String, Child)>>> = Arc::new(Mutex::new(Vec::new()));

    {
        let mut procs = processes.lock().unwrap();
        for node_name in &node_names {
            let node_name = node_name.to_string();
            let node_config = match config.nodes.get(&node_name) {
                Some(n) => n,
                None => {
                    eprintln!("  {} Node \"{}\" not found", "✗".red().bold(), node_name);
                    continue;
                }
            };

            println!(
                "  {} {}  {}",
                "▶".green().bold(),
                format!("[{}]", node_name).cyan().bold(),
                node_config.script_rel.dimmed()
            );

            // Build the command:
            // python3 /tmp/reachpy-launcher.py <script> <name> [--ros-args ...]
            let mut cmd = build_node_command(
                &launcher_path,
                &node_config.script,
                &node_name,
                &config,
            );

            match cmd.spawn() {
                Ok(child) => {
                    procs.push((node_name, child));
                }
                Err(e) => {
                    eprintln!(
                        "  {} Failed to start [{}]: {}",
                        "✗".red().bold(), node_name, e
                    );
                }
            }
        }
    }

    println!();
    println!("  {} All nodes running. Press {} to stop.", "✓".green().bold(), "Ctrl+C".cyan());
    println!();

    // Ctrl+C handler — clean shutdown
    let processes_ctrlc = Arc::clone(&processes);
    let launcher_path_ctrlc = launcher_path.clone();
    ctrlc::set_handler(move || {
        println!();
        println!("  {} Shutting down all nodes...", "■".yellow().bold());
        let mut procs = processes_ctrlc.lock().unwrap();
        for (name, child) in procs.iter_mut() {
            let _ = child.kill();
            let _ = child.wait();
            println!("  {} [{}] stopped", "✓".green(), name.cyan());
        }
        // Clean up temp launcher
        let _ = std::fs::remove_file(&launcher_path_ctrlc);
        println!("  {} Goodbye.", "✓".green().bold());
        std::process::exit(0);
    }).expect("Failed to set Ctrl-C handler");

    // Supervisor loop — watch for crashed nodes
    // Supervisor loop
    let mut dead: std::collections::HashSet<String> = std::collections::HashSet::new();
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let mut procs = processes.lock().unwrap();
        for (name, child) in procs.iter_mut() {
            if dead.contains(name) { continue; }
            match child.try_wait() {
                Ok(Some(status)) => {
                    dead.insert(name.clone());
                    if !status.success() {
                        println!(
                            "  {} [{}] exited with error. Check output above.",
                            "!".red().bold(),
                            name.cyan()
                        );
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("  {} Error watching [{}]: {}", "✗".red(), name, e);
                }
            }
        }
        // If all nodes are dead, exit
        if dead.len() == procs.len() {
            println!("  {} All nodes have stopped.", "■".yellow().bold());
            std::process::exit(0);
        }
    }
}

// ─── Helpers for reach run ────────────────────────────────────────────────────

/// Write the embedded launcher.py to /tmp and return its path
fn write_launcher() -> Result<PathBuf, std::io::Error> {
    let path = std::env::temp_dir().join("reachpy-launcher.py");
    let mut file = std::fs::File::create(&path)?;
    file.write_all(LAUNCHER_PY.as_bytes())?;
    Ok(path)
}

/// Find the ROS2 setup.bash — checks common install locations
fn find_ros2_setup() -> Option<PathBuf> {
    // Check if already sourced via env var
    if let Ok(ament_prefix) = std::env::var("AMENT_PREFIX_PATH") {
        if !ament_prefix.is_empty() {
            // ROS2 already sourced — find the setup path from prefix
            let first = ament_prefix.split(':').next().unwrap_or("");
            let setup = PathBuf::from(first).join("../setup.bash");
            if setup.exists() {
                return Some(setup);
            }
            // Return a sentinel — we know ROS2 is sourced even if we can't pinpoint setup.bash
            return Some(PathBuf::from(first));
        }
    }

    // Common ROS2 install locations
    let candidates = [
        "/opt/ros/humble/setup.bash",
        "/opt/ros/iron/setup.bash",
        "/opt/ros/jazzy/setup.bash",
        "/opt/ros/rolling/setup.bash",
    ];

    for candidate in &candidates {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return Some(path);
        }
    }

    None
}

/// Build the full command to launch a single node
fn build_node_command(
    launcher: &PathBuf,
    script: &PathBuf,
    node_name: &str,
    config: &ReachConfig,
) -> Command {
    let mut cmd = Command::new("python3");

    cmd.arg(launcher)
       .arg(script)
       .arg(node_name);

    // Set ROS_DOMAIN_ID from reach.toml
    cmd.env("ROS_DOMAIN_ID", config.robot.domain_id.to_string());

    // Build --ros-args if we have remappings or node-specific args
    let mut ros_args: Vec<String> = Vec::new();

    // Global remappings from [ros.remappings]
    for (from, to) in &config.ros.remappings {
        ros_args.push("-r".to_string());
        ros_args.push(format!("{}:={}", from, to));
    }

    // Per-node args from [ros.node_args]
    if let Some(extra) = config.ros.node_args.get(node_name) {
        ros_args.extend(extra.clone());
    }

    // Namespace from [ros]
    if let Some(ref ns) = config.ros.namespace {
        ros_args.push("--ros-args".to_string());
        ros_args.push("-r".to_string());
        ros_args.push(format!("__ns:={}", ns));
    }

    if !ros_args.is_empty() {
        cmd.arg("--ros-args");
        for arg in ros_args {
            cmd.arg(arg);
        }
    }

    // Inherit stdout/stderr so node output appears in terminal
    cmd.stdout(std::process::Stdio::inherit())
       .stderr(std::process::Stdio::inherit());

    cmd
}

// ─── Stubs ────────────────────────────────────────────────────────────────────

fn cmd_coming_soon(cmd: &str) {
    println!();
    println!("  {} {} is coming soon.", "→".bright_blue().bold(), format!("reach {}", cmd).cyan().bold());
    println!("  Follow {} for updates.", "ascii-robotics.com".white());
    println!();
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn fatal(msg: &str) -> ! {
    eprintln!("  {} {}", "✗".red().bold(), msg.red());
    std::process::exit(1);
}

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("create") => {
            match args.get(2) {
                Some(name) => cmd_create(name),
                None => {
                    eprintln!("  {} Usage: {}", "✗".red().bold(), "reach create <workspace-name>".cyan());
                    std::process::exit(1);
                }
            }
        }
        Some("config") => cmd_config(),
        Some("run")    => {
            let profile = args.get(2).map(|s| s.as_str()).unwrap_or("default");
            cmd_run(profile);
        }
        Some("dev")    => cmd_coming_soon("dev"),
        Some("build")  => cmd_coming_soon("build"),
        Some("doctor") => cmd_coming_soon("doctor"),
        Some(unknown)  => {
            eprintln!("  {} Unknown command \"{}\". Run {} for help.", "✗".red().bold(), unknown, "reach".cyan());
            std::process::exit(1);
        }
        None => print_help(),
    }
}