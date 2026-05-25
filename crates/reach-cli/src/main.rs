use colored::Colorize;
use reachpy_config::{ConfigError, ReachConfig};
use std::path::Path;

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
    println!("  {}  {}  {}", "reach create".cyan().bold(), "<name>".white(), "Create a new ReachPy workspace".dimmed());
    println!("  {}          {}",  "reach config".cyan().bold(),              "Show resolved config for current workspace".dimmed());
    println!("  {}    {}",        "reach dev".cyan().bold(),                 "Start hot-reload dev server  (coming soon)".dimmed());
    println!("  {}    {}",        "reach run".cyan().bold(),                 "Run a launch profile          (coming soon)".dimmed());
    println!("  {}  {}",          "reach build".cyan().bold(),               "Bundle for production         (coming soon)".dimmed());
    println!("  {}  {}",          "reach doctor".cyan().bold(),              "Diagnose workspace issues     (coming soon)".dimmed());
    println!();
}

// ─── reach create ─────────────────────────────────────────────────────────────

fn cmd_create(name: &str) {
    let workspace = Path::new(name);

    // Don't clobber existing dirs
    if workspace.exists() {
        eprintln!(
            "  {} Directory \"{}\" already exists.",
            "✗".red().bold(), name
        );
        std::process::exit(1);
    }

    println!();
    println!(
        "  {} Creating ReachPy workspace {}...",
        "→".bright_blue().bold(),
        name.white().bold()
    );

    // Create directory structure
    let dirs = ["src", "config", "models", "launch"];
    for dir in &dirs {
        std::fs::create_dir_all(workspace.join(dir))
            .unwrap_or_else(|e| fatal(&format!("Failed to create {}: {}", dir, e)));
    }

    // Write reach.toml
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
platform = "generic"     # e.g. ur5, spot, custom
transport = "ros2"       # ros2 (ros1 coming soon)
domain_id = 0            # ROS_DOMAIN_ID

# ── Nodes ───────────────────────────────────
# Each entry is: node_name = "path/to/script.py"
# ReachPy compiles these to ROS2 nodes automatically.
# No CMakeLists, no setup.py, no entry_points.

[nodes]
example = "src/example.py"

# ── Launch profiles ─────────────────────────
# Replaces XML launch files entirely.
# `reach run` uses "default". `reach run <profile>` runs a named profile.
# Each profile is a list of node names from [nodes] above.

[launch]
default = ["example"]
# camera = ["detector", "preprocessor"]
# full = ["detector", "preprocessor", "controller"]

# ── Dependencies ────────────────────────────
# ReachPy handles installation. No setup.py required.
# [dependencies]
# opencv = "4.8"
# torch = "2.0"
# numpy = "*"

# ── Dev settings ────────────────────────────
[dev]
hot_reload = true
hot_reload_ignore = [
    "config/",    # robot config files — don't hot-swap mid-run
    "models/",    # ML model files — too large and stateful
]

# ── ROS escape hatch ────────────────────────
# For advanced ROS2 features that reach.toml abstracts.
# 99% of projects won't need this section.
#
# [ros]
# namespace = "/robot1"
#
# [ros.remappings]
# "/camera/raw" = "/camera/compressed"
#
# [ros.node_args]
# detector = ["--ros-args", "--log-level", "DEBUG"]
"#,
        name = name
    );

    std::fs::write(workspace.join("reach.toml"), toml)
        .unwrap_or_else(|e| fatal(&format!("Failed to write reach.toml: {}", e)));

    // Write example node
    let example_node = r#"# ─────────────────────────────────────────────
#  example.py — ReachPy node
#
#  Write your robot logic here.
#  ReachPy compiles this to a ROS2 node automatically.
#  No boilerplate. No class inheritance. No spin loops.
# ─────────────────────────────────────────────

from reachpy import node, Topic

# Uncomment when ReachPy runtime is installed:
# from std_msgs.msg import String

# @node
# def example(input: Topic[String]) -> Topic[String]:
#     """Echo node — receives a message and publishes it back."""
#     return input

# For now, a plain Python placeholder:
import time

print("[example] Node started — make your code reach the robot.")
while True:
    print("[example] Running...")
    time.sleep(1)
"#;

    std::fs::write(workspace.join("src/example.py"), example_node)
        .unwrap_or_else(|e| fatal(&format!("Failed to write example node: {}", e)));

    // Write .gitignore
    let gitignore = "# ReachPy build artifacts\n.reach/\ndist/\n__pycache__/\n*.pyc\n*.pyo\n\n# Models (usually too large for git)\nmodels/\n\n# Environment\n.env\n";
    std::fs::write(workspace.join(".gitignore"), gitignore)
        .unwrap_or_else(|e| fatal(&format!("Failed to write .gitignore: {}", e)));

    // Write README
    let readme = format!(
"# {name}\n\nA ReachPy robotics workspace.\n\n## Getting started\n\n```bash\n# Start the dev server with hot reload\nreach dev\n\n# Run the default launch profile\nreach run\n\n# Run a named launch profile\nreach run camera\n\n# Build for production\nreach build\n```\n\n## Project structure\n\n```\n{name}/\n├── reach.toml        # Everything. Replaces CMakeLists, package.xml, launch files.\n├── src/              # Your node scripts\n│   └── example.py\n├── config/           # Robot config files (not hot-reloaded)\n├── models/           # ML models (not hot-reloaded)\n└── launch/           # Optional custom launch logic\n```\n\nBuilt with [ReachPy](https://ascii-robotics.com) — make your code reach the robot.\n",
        name = name
    );
    std::fs::write(workspace.join("README.md"), readme)
        .unwrap_or_else(|e| fatal(&format!("Failed to write README: {}", e)));

    // Success output
    println!();
    println!("  {} {}", "✓".green().bold(), format!("Created workspace \"{}\"", name).white().bold());
    println!();
    println!("  {}", "Structure:".dimmed());
    println!("  {}  {}", "├── reach.toml".cyan(),       "← your entire config lives here".dimmed());
    println!("  {}  {}", "├── src/".white(),             "← node scripts".dimmed());
    println!("  {}  {}", "│   └── example.py".white(),  "← example node".dimmed());
    println!("  {}  {}", "├── config/".white(),          "← robot config (not hot-reloaded)".dimmed());
    println!("  {}  {}", "├── models/".white(),          "← ML models (not hot-reloaded)".dimmed());
    println!("  {}  {}", "├── .gitignore".dimmed(),     "".dimmed());
    println!("  {}  {}", "└── README.md".dimmed(),      "".dimmed());
    println!();
    println!("  {}", "Next steps:".dimmed());
    println!("  {}  {}", "cd".cyan(), name.white().bold());
    println!("  {}", "reach dev".cyan().bold());
    println!();
    println!(
        "  {} Make your code reach the robot.",
        "→".bright_blue().bold()
    );
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

// ─── Stubs for coming soon commands ──────────────────────────────────────────

fn cmd_coming_soon(cmd: &str) {
    println!();
    println!(
        "  {} {} is coming soon.",
        "→".bright_blue().bold(),
        format!("reach {}", cmd).cyan().bold()
    );
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
        Some("dev")    => cmd_coming_soon("dev"),
        Some("run")    => cmd_coming_soon("run"),
        Some("build")  => cmd_coming_soon("build"),
        Some("doctor") => cmd_coming_soon("doctor"),
        Some(unknown)  => {
            eprintln!("  {} Unknown command \"{}\". Run {} for help.", "✗".red().bold(), unknown, "reach".cyan());
            std::process::exit(1);
        }
        None => print_help(),
    }
}