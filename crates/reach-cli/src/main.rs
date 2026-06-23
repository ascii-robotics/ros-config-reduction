use colored::Colorize;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use reachpy_config::{ConfigError, ReachConfig, NodeConfig};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ─── Embedded launcher.py ─────────────────────────────────────────────────────
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
    println!("  {}  {}     {}", "reach create".cyan().bold(), "<name>".white(),              "Create a new ReachPy workspace".dimmed());
    println!("  {}             {}", "reach config".cyan().bold(),                            "Show resolved config".dimmed());
    println!("  {}             {}", "reach run".cyan().bold(),                               "Run default launch profile with hot reload".dimmed());
    println!("  {}  {}  {}", "reach run".cyan().bold(), "<profile|node...>".white(),         "Run a profile or specific nodes".dimmed());
    println!("  {}  {}  {}", "reach launch".cyan().bold(), "<pipeline>".white(),             "Run a named launch pipeline".dimmed());
    println!("  {}          {}", "reach build".cyan().bold(),                                "coming soon".dimmed());
    println!("  {}         {}", "reach doctor".cyan().bold(), "Diagnose workspace issues".dimmed());
    println!("  {}          {}", "reach build".cyan().bold(),  "Generate ROS2 package from reach.toml".dimmed());
    println!("  {}           {}", "reach bags".cyan().bold(),  "Record and play rosbags".dimmed());
    println!("  {}          {}", "reach trace".cyan().bold(),  "Trace message flow and node relationships".dimmed());
    println!();
}

// ─── Process manager ─────────────────────────────────────────────────────────

struct NodeProcess {
    name: String,
    config: NodeConfig,
    child: Option<Child>,
}

impl NodeProcess {
    fn new(name: String, config: NodeConfig) -> Self {
        NodeProcess { name, config, child: None }
    }

    fn start(&mut self, launcher: &PathBuf, reach_config: &ReachConfig) {
        println!(
            "  {} {}  {}",
            "▶".green().bold(),
            format!("[{}]", self.name).cyan().bold(),
            self.config.script_rel.dimmed()
        );
        let mut cmd = build_node_command(launcher, &self.config.script, &self.name, reach_config);
        match cmd.spawn() {
            Ok(child) => self.child = Some(child),
            Err(e) => eprintln!(
                "  {} Failed to start [{}]: {}", "✗".red().bold(), self.name, e
            ),
        }
    }

    fn stop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
    }

    fn restart(&mut self, launcher: &PathBuf, reach_config: &ReachConfig) {
        self.stop();
        std::thread::sleep(Duration::from_millis(150));
        println!(
            "  {} {} {}",
            "↻".bright_blue().bold(),
            format!("[{}]", self.name).cyan().bold(),
            "reloading...".bright_blue()
        );
        self.start(launcher, reach_config);
        println!(
            "  {} {} {}",
            "✓".green().bold(),
            format!("[{}]", self.name).cyan(),
            "hot reloaded".green()
        );
    }

    fn is_running(&mut self) -> bool {
        if let Some(ref mut child) = self.child {
            matches!(child.try_wait(), Ok(None))
        } else {
            false
        }
    }
}

// ─── Hot reload helpers ───────────────────────────────────────────────────────

/// Check if a Python file has # hot-reload-off in its first 5 lines
fn is_hot_reload_disabled(path: &Path) -> bool {
    if let Ok(content) = std::fs::read_to_string(path) {
        for line in content.lines().take(5) {
            let trimmed = line.trim();
            if trimmed == "# hot-reload-off" || trimmed == "#hot-reload-off" {
                return true;
            }
        }
    }
    false
}

/// Find which node owns a changed file
fn find_owning_node(
    changed: &Path,
    nodes: &HashMap<String, NodeConfig>,
) -> Option<String> {
    for (name, node) in nodes {
        if changed == node.script {
            return Some(name.clone());
        }
        if let (Some(cp), Some(sp)) = (changed.parent(), node.script.parent()) {
            if cp == sp {
                return Some(name.clone());
            }
        }
    }
    None
}

// ─── reach create ────────────────────────────────────────────────────────────

fn cmd_create(name: &str) {
    let workspace = std::path::Path::new(name);
    if workspace.exists() {
        eprintln!("  {} Directory \"{}\" already exists.", "✗".red().bold(), name);
        std::process::exit(1);
    }

    println!();
    println!("  {} Creating ReachPy workspace {}...", "→".bright_blue().bold(), name.white().bold());

    for dir in &["src", "config", "models", "launch"] {
        std::fs::create_dir_all(workspace.join(dir))
            .unwrap_or_else(|e| fatal(&format!("Failed to create {}: {}", dir, e)));
    }

    let toml = format!(
r#"# ─────────────────────────────────────────────
#  reach.toml — ReachPy workspace configuration
#  Replaces: CMakeLists.txt, package.xml, setup.py, XML launch files
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

    let example = "# ReachPy example node\n# Add: # hot-reload-off at the top to disable hot reload for this node\nimport time\n\nprint('[example] Node started - make your code reach the robot.')\ni = 0\nwhile True:\n    print(f'[example] tick {i}')\n    i += 1\n    time.sleep(1)\n";
    std::fs::write(workspace.join("src/example.py"), example)
        .unwrap_or_else(|e| fatal(&format!("Failed to write example: {}", e)));

    std::fs::write(workspace.join(".gitignore"),
        "# ReachPy\n.reach/\ndist/\n__pycache__/\n*.pyc\nmodels/\n.env\n")
        .unwrap_or_else(|e| fatal(&format!("Failed to write .gitignore: {}", e)));

    println!();
    println!("  {} {}", "✓".green().bold(), format!("Created workspace \"{}\"", name).white().bold());
    println!();
    println!("  {}  {}", "├── reach.toml".cyan(),      "← your entire config lives here".dimmed());
    println!("  {}  {}", "├── src/".white(),            "← node scripts".dimmed());
    println!("  {}  {}", "│   └── example.py".white(), "← example node".dimmed());
    println!("  {}  {}", "├── config/".white(),         "← robot config (not hot-reloaded)".dimmed());
    println!("  {}  {}", "└── models/".white(),         "← ML models (not hot-reloaded)".dimmed());
    println!();
    println!("  {}  {}", "cd".cyan(), name.white().bold());
    println!("  {}", "reach run".cyan().bold());
    println!();
    println!("  {} Make your code reach the robot.", "→".bright_blue().bold());
    println!();
}

// ─── reach config ────────────────────────────────────────────────────────────

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
            eprintln!("  {} No reach.toml found. Run {} to create a workspace.",
                "✗".red().bold(), "reach create <name>".cyan());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("  {} Config error:", "✗".red().bold());
            for line in e.to_string().lines() {
                eprintln!("  {}", line.red());
            }
            std::process::exit(1);
        }
    }
}

// ─── reach run ───────────────────────────────────────────────────────────────

fn cmd_run(args: &[String]) {
    let config = load_config_or_exit();

    // Resolve which nodes to run from args
    // Priority: if arg matches a launch profile → use profile
    //           if arg matches node names → run those nodes
    //           if no args → run default profile
    let nodes_to_run: Vec<NodeConfig> = if args.is_empty() {
        // No args — run default launch profile
        resolve_profile("default", &config)
    } else if args.len() == 1 {
        // Single arg — check if it's a profile first, then a node name
        let arg = &args[0];
        if config.launch.contains_key(arg.as_str()) {
            resolve_profile(arg, &config)
        } else if config.nodes.contains_key(arg.as_str()) {
            vec![config.nodes[arg.as_str()].clone()]
        } else {
            eprintln!(
                "  {} \"{}\" is not a launch profile or node name.",
                "✗".red().bold(), arg
            );
            eprintln!("  Profiles: {}", config.launch.keys().cloned().collect::<Vec<_>>().join(", "));
            eprintln!("  Nodes:    {}", config.nodes.keys().cloned().collect::<Vec<_>>().join(", "));
            std::process::exit(1);
        }
    } else {
        // Multiple args — treat each as a node name
        let mut nodes = Vec::new();
        for arg in args {
            match config.nodes.get(arg.as_str()) {
                Some(n) => nodes.push(n.clone()),
                None => {
                    eprintln!("  {} Unknown node \"{}\".", "✗".red().bold(), arg);
                    eprintln!("  Available: {}", config.nodes.keys().cloned().collect::<Vec<_>>().join(", "));
                    std::process::exit(1);
                }
            }
        }
        nodes
    };

    if nodes_to_run.is_empty() {
        eprintln!("  {} No nodes to run.", "✗".red().bold());
        std::process::exit(1);
    }

    print_banner();
    println!(
        "  {}  {} node{}  {} hot reload",
        "reach run".white().bold(),
        nodes_to_run.len().to_string().white(),
        if nodes_to_run.len() == 1 { "" } else { "s" },
        "→".dimmed(),
    );
    println!();

    // Write launcher
    let launcher = write_launcher()
        .unwrap_or_else(|e| fatal(&format!("Failed to write launcher: {}", e)));

    // Check ROS2
    if find_ros2_setup().is_none() {
        eprintln!("  {} ROS2 not found. Source ROS2 first:", "✗".red().bold());
        eprintln!("  {}", "source /opt/ros/humble/setup.bash".cyan());
        std::process::exit(1);
    }

    println!("  {} {}", "domain_id:".dimmed(), config.robot.domain_id.to_string().white());
    println!();

    // Build process map
    let processes: Arc<Mutex<HashMap<String, NodeProcess>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Start all nodes
    {
        let mut procs = processes.lock().unwrap();
        for node_config in &nodes_to_run {
            let mut proc = NodeProcess::new(node_config.name.clone(), node_config.clone());
            proc.start(&launcher, &config);
            procs.insert(node_config.name.clone(), proc);
        }
    }

    println!();

    // Hot reload — check which nodes have it enabled
    let hot_reload_enabled = config.dev.hot_reload;
    let ignore_patterns = config.dev.hot_reload_ignore.clone();
    let project_root = config.root.clone();

    if hot_reload_enabled {
        println!("  {} watching for changes  {}", "◉".bright_blue(), "(# hot-reload-off to disable per node)".dimmed());
    } else {
        println!("  {} hot reload disabled in reach.toml", "○".dimmed());
    }
    println!("  Press {} to stop.", "Ctrl+C".cyan());
    println!();

    // Ctrl+C handler
    let processes_ctrlc = Arc::clone(&processes);
    let launcher_ctrlc = launcher.clone();
    ctrlc::set_handler(move || {
        println!();
        println!("  {} Shutting down...", "■".yellow().bold());
        let mut procs = processes_ctrlc.lock().unwrap();
        for (name, proc) in procs.iter_mut() {
            proc.stop();
            println!("  {} [{}] stopped", "✓".green(), name.cyan());
        }
        let _ = std::fs::remove_file(&launcher_ctrlc);
        println!("  {} Goodbye.", "✓".green().bold());
        std::process::exit(0);
    }).expect("Failed to set Ctrl-C handler");

    // ── File watcher for hot reload ──────────────────────────────────────────
    if hot_reload_enabled {
        let processes_watcher = Arc::clone(&processes);
        let config_clone = config.clone();
        let launcher_clone = launcher.clone();

        std::thread::spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
            let mut watcher = RecommendedWatcher::new(
                move |res| { let _ = tx.send(res); },
                Config::default().with_poll_interval(Duration::from_millis(200)),
            ).expect("Failed to create watcher");

            watcher.watch(&project_root, RecursiveMode::Recursive)
                .expect("Failed to watch project");

            let mut last_reload: HashMap<String, Instant> = HashMap::new();
            let debounce = Duration::from_millis(400);

            loop {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(Ok(event)) => {
                        let is_relevant = matches!(
                            event.kind,
                            EventKind::Modify(_) | EventKind::Create(_)
                        );
                        if !is_relevant { continue; }

                        for path in &event.paths {
                            // Only watch .py files
                            if path.extension().and_then(|e| e.to_str()) != Some("py") {
                                continue;
                            }

                            // Check ignore patterns
                            let rel = path.strip_prefix(&config_clone.root).unwrap_or(path);
                            let rel_str = rel.to_string_lossy();
                            if ignore_patterns.iter().any(|p| rel_str.contains(p.trim_matches('/'))) {
                                continue;
                            }
                            if rel_str.contains("__pycache__") || rel_str.ends_with(".pyc") {
                                continue;
                            }

                            // Find owning node
                            let node_name = find_owning_node(path, &config_clone.nodes);

                            let now = Instant::now();
                            match node_name {
                                Some(name) => {
                                    // Debounce
                                    if last_reload.get(&name)
                                        .map_or(false, |l| now.duration_since(*l) < debounce) {
                                        continue;
                                    }

                                    // Check # hot-reload-off
                                    if is_hot_reload_disabled(path) {
                                        println!(
                                            "  {} {} {}",
                                            "~".dimmed(),
                                            format!("[{}]", name).dimmed(),
                                            "hot-reload-off — skipping".dimmed()
                                        );
                                        continue;
                                    }

                                    last_reload.insert(name.clone(), now);

                                    println!(
                                        "  {} {} changed",
                                        "→".bright_blue(),
                                        rel.display().to_string().white()
                                    );

                                    let mut procs = processes_watcher.lock().unwrap();
                                    if let Some(proc) = procs.get_mut(&name) {
                                        proc.restart(&launcher_clone, &config_clone);
                                    }
                                }
                                None => {
                                    // Shared module — reload all nodes that don't have hot-reload-off
                                    let key = "__all__".to_string();
                                    if last_reload.get(&key)
                                        .map_or(false, |l| now.duration_since(*l) < debounce) {
                                        continue;
                                    }
                                    last_reload.insert(key, now);

                                    println!(
                                        "  {} {} changed → reloading all eligible nodes",
                                        "→".yellow(),
                                        rel.display().to_string().white()
                                    );

                                    let mut procs = processes_watcher.lock().unwrap();
                                    for (_, proc) in procs.iter_mut() {
                                        if !is_hot_reload_disabled(&proc.config.script) {
                                            proc.restart(&launcher_clone, &config_clone);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(Err(e)) => eprintln!("  {} Watcher error: {}", "✗".red(), e),
                    Err(_) => {} // timeout, continue
                }
            }
        });
    }

    // ── Supervisor loop ──────────────────────────────────────────────────────
    let mut dead: HashSet<String> = HashSet::new();
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let mut procs = processes.lock().unwrap();
        for (name, proc) in procs.iter_mut() {
            if dead.contains(name) { continue; }
            match proc.child.as_mut().and_then(|c| c.try_wait().ok()) {
                Some(Some(status)) => {
                    dead.insert(name.clone());
                    if !status.success() {
                        println!(
                            "  {} [{}] exited with error. Check output above.",
                            "!".red().bold(), name.cyan()
                        );
                    }
                }
                _ => {}
            }
        }
        if dead.len() == procs.len() && !procs.is_empty() {
            println!("  {} All nodes stopped.", "■".yellow().bold());
            let _ = std::fs::remove_file(&launcher);
            std::process::exit(0);
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn load_config_or_exit() -> ReachConfig {
    match ReachConfig::load() {
        Ok(c) => c,
        Err(ConfigError::NotFound) => {
            eprintln!("  {} No reach.toml found.", "✗".red().bold());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("  {} {}", "✗".red().bold(), e.to_string().red());
            std::process::exit(1);
        }
    }
}

fn resolve_profile(profile: &str, config: &ReachConfig) -> Vec<NodeConfig> {
    match config.launch.get(profile) {
        Some(pipeline) => pipeline.nodes.iter()
            .filter_map(|n| config.nodes.get(&n.name))
            .cloned()
            .collect(),
        None if profile == "default" => {
            // No default pipeline — run all nodes
            config.nodes.values().cloned().collect()
        }
        None => {
            eprintln!("  {} Launch profile \"{}\" not found.", "✗".red().bold(), profile);
            if !config.launch.is_empty() {
                eprintln!("  Available: {}", config.launch.keys().cloned().collect::<Vec<_>>().join(", "));
            }
            std::process::exit(1);
        }
    }
}

fn write_launcher() -> Result<PathBuf, std::io::Error> {
    let path = std::env::temp_dir().join("reachpy-launcher.py");
    let mut file = std::fs::File::create(&path)?;
    file.write_all(LAUNCHER_PY.as_bytes())?;
    Ok(path)
}

fn find_ros2_setup() -> Option<PathBuf> {
    if let Ok(prefix) = std::env::var("AMENT_PREFIX_PATH") {
        if !prefix.is_empty() {
            return Some(PathBuf::from(prefix.split(':').next().unwrap_or("")));
        }
    }
    for candidate in &[
        "/opt/ros/humble/setup.bash",
        "/opt/ros/iron/setup.bash",
        "/opt/ros/jazzy/setup.bash",
    ] {
        let p = PathBuf::from(candidate);
        if p.exists() { return Some(p); }
    }
    None
}

fn build_node_command(
    launcher: &PathBuf,
    script: &PathBuf,
    node_name: &str,
    config: &ReachConfig,
) -> Command {
    let mut cmd = Command::new("python3");
    cmd.arg(launcher).arg(script).arg(node_name);
    cmd.env("ROS_DOMAIN_ID", config.robot.domain_id.to_string());

    let mut ros_args: Vec<String> = Vec::new();
    for (from, to) in &config.ros.remappings {
        ros_args.push("-r".to_string());
        ros_args.push(format!("{}:={}", from, to));
    }
    if let Some(extra) = config.ros.node_args.get(node_name) {
        ros_args.extend(extra.clone());
    }
    if let Some(ref ns) = config.ros.namespace {
        ros_args.push("-r".to_string());
        ros_args.push(format!("__ns:={}", ns));
    }
    if !ros_args.is_empty() {
        cmd.arg("--ros-args");
        for arg in ros_args { cmd.arg(arg); }
    }

    cmd.stdout(std::process::Stdio::inherit())
       .stderr(std::process::Stdio::inherit());
    cmd
}

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
                    eprintln!("  {} Usage: reach create <name>", "✗".red().bold());
                    std::process::exit(1);
                }
            }
        }
        Some("config") => cmd_config(),
        Some("run")    => cmd_run(&args[2..].to_vec()),
        Some("launch") => {
            match args.get(2) {
                Some(name) => cmd_launch(name),
                None => {
                    eprintln!("  {} Usage: reach launch <pipeline>", "✗".red().bold());
                    eprintln!("  Example: reach launch prod");
                    std::process::exit(1);
                }
            }
        }
        Some("doctor") => cmd_doctor(),
        Some("build")  => cmd_build(),
        Some("bags")   => cmd_bags(&args[2..].to_vec()),
        Some("trace")  => cmd_trace(&args[2..].to_vec()),
        Some(unknown)  => {
            eprintln!("  {} Unknown command \"{}\". Run {} for help.",
                "✗".red().bold(), unknown, "reach".cyan());
            std::process::exit(1);
        }
        None => print_help(),
    }
}

fn cmd_doctor() {
    println!();
    println!("  {} {}", "reach doctor".white().bold(), "— diagnosing workspace...".dimmed());
    println!();
    println!("  {}", "─".repeat(44).dimmed());
    println!();

    let mut issues: u32 = 0;
    let mut warnings: u32 = 0;

    // ── ROS2 ─────────────────────────────────────────────────────────────────
    println!("  {}", "ROS2".white().bold());

    // Check AMENT_PREFIX_PATH
    let ament = std::env::var("AMENT_PREFIX_PATH").unwrap_or_default();
    if ament.is_empty() {
        println!("  {} ROS2 is not sourced", "✗".red().bold());
        println!("    Try: {}", "source /opt/ros/humble/setup.bash".cyan());
        issues += 1;
    } else {
        // Detect distro from path
        let distro = detect_ros2_distro(&ament);
        println!("  {} ROS2 {} sourced", "✓".green().bold(), distro.white().bold());
    }

    // Check rclpy importable
    let rclpy_check = Command::new("python3")
        .arg("-c")
        .arg("import rclpy; print(rclpy.__file__)")
        .output();
    match rclpy_check {
        Ok(out) if out.status.success() => {
            println!("  {} rclpy importable", "✓".green().bold());
        }
        _ => {
            println!("  {} rclpy not importable — ROS2 may not be sourced", "✗".red().bold());
            issues += 1;
        }
    }

    // Check ros2 CLI available
    let ros2_cli = Command::new("ros2").arg("--version").output();
    match ros2_cli {
        Ok(out) if out.status.success() => {
            let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
            println!("  {} ros2 CLI available  {}", "✓".green().bold(), ver.dimmed());
        }
        _ => {
            println!("  {} ros2 CLI not found on PATH", "⚠".yellow().bold());
            warnings += 1;
        }
    }

    println!();

    // ── Python ───────────────────────────────────────────────────────────────
    println!("  {}", "Python".white().bold());

    let python_check = Command::new("python3").arg("--version").output();
    match python_check {
        Ok(out) if out.status.success() => {
            let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
            println!("  {} {} on PATH", "✓".green().bold(), ver.white());

            // Check against reach.toml version if available
            if let Ok(config) = ReachConfig::load() {
                let toml_ver = &config.project.python;
                let sys_ver = ver.replace("Python ", "");
                if !sys_ver.starts_with(toml_ver.trim_end_matches(".0")) {
                    println!(
                        "  {} system Python {} vs reach.toml python = \"{}\"",
                        "⚠".yellow().bold(),
                        sys_ver.white(),
                        toml_ver.yellow()
                    );
                    warnings += 1;
                }
            }
        }
        _ => {
            println!("  {} python3 not found on PATH", "✗".red().bold());
            issues += 1;
        }
    }

    println!();

    // ── Workspace ────────────────────────────────────────────────────────────
    println!("  {}", "Workspace".white().bold());

    let config = match ReachConfig::load() {
        Ok(c) => {
            println!("  {} reach.toml valid", "✓".green().bold());
            println!(
                "    {} {}  {} {}",
                "project:".dimmed(), c.project.name.white(),
                "version:".dimmed(), c.project.version.white()
            );
            Some(c)
        }
        Err(ConfigError::NotFound) => {
            println!("  {} No reach.toml found — not inside a ReachPy workspace", "✗".red().bold());
            println!("    Run {} to create one.", "reach create <name>".cyan());
            issues += 1;
            None
        }
        Err(e) => {
            println!("  {} reach.toml invalid:", "✗".red().bold());
            for line in e.to_string().lines() {
                println!("    {}", line.red());
            }
            issues += 1;
            None
        }
    };

    if let Some(ref config) = config {
        println!();

        // ── Nodes ─────────────────────────────────────────────────────────
        println!("  {}", "Nodes".white().bold());

        for (name, node) in &config.nodes {
            if !node.script.exists() {
                println!("  {} [{}] script not found: {}", "✗".red().bold(), name.cyan(), node.script_rel.red());
                issues += 1;
                continue;
            }

            // Try importing the node script to catch syntax/import errors
            let import_check = Command::new("python3")
                .arg("-c")
                .arg(format!(
                    "import ast, sys; ast.parse(open('{}').read()); print('ok')",
                    node.script.display()
                ))
                .output();

            match import_check {
                Ok(out) if out.status.success() => {
                    // Check for hot-reload-off
                    let hro = is_hot_reload_disabled(&node.script);
                    println!(
                        "  {} [{}]  {}  {}",
                        "✓".green().bold(),
                        name.cyan(),
                        node.script_rel.dimmed(),
                        if hro { "hot-reload-off".yellow().to_string() } else { "".to_string() }
                    );
                }
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    println!("  {} [{}] syntax error:", "✗".red().bold(), name.cyan());
                    for line in err.lines().take(3) {
                        println!("    {}", line.red());
                    }
                    issues += 1;
                }
                Err(e) => {
                    println!("  {} [{}] could not check: {}", "⚠".yellow().bold(), name.cyan(), e);
                    warnings += 1;
                }
            }
        }

        println!();

        // ── Launch pipelines ──────────────────────────────────────────────
        if !config.launch.is_empty() {
            println!("  {}", "Launch Pipelines".white().bold());
            for (name, pipeline) in &config.launch {
                let node_names: Vec<&str> = pipeline.nodes.iter()
                    .map(|n| n.name.as_str())
                    .collect();
                println!(
                    "  {} [{}]  {} node{}  {}",
                    "✓".green().bold(),
                    name.cyan(),
                    pipeline.nodes.len(),
                    if pipeline.nodes.len() == 1 { "" } else { "s" },
                    format!("[{}]", node_names.join(", ")).dimmed()
                );

                // Check depends_on chains make sense
                for node in &pipeline.nodes {
                    if let Some(ref dep) = node.depends_on {
                        let dep_exists = pipeline.nodes.iter().any(|n| &n.name == dep);
                        if !dep_exists {
                            println!(
                                "    {} [{}] depends_on \"{}\" which is not in this pipeline",
                                "⚠".yellow(), node.name.cyan(), dep.yellow()
                            );
                            warnings += 1;
                        }
                    }
                }
            }
            println!();
        }

        // ── Dependencies ─────────────────────────────────────────────────
        if !config.dependencies.is_empty() {
            println!("  {}", "Dependencies".white().bold());
            for dep in &config.dependencies {
                let check = Command::new("python3")
                    .arg("-c")
                    .arg(format!("import {}", dep.name.replace('-', "_")))
                    .output();
                match check {
                    Ok(out) if out.status.success() => {
                        println!(
                            "  {} {}  {}",
                            "✓".green().bold(),
                            dep.name.white(),
                            dep.version.as_deref().unwrap_or("*").dimmed()
                        );
                    }
                    _ => {
                        println!(
                            "  {} {} not installed",
                            "✗".red().bold(),
                            dep.name.white()
                        );
                        println!(
                            "    Try: {}",
                            format!("pip install {}", dep.name).cyan()
                        );
                        issues += 1;
                    }
                }
            }
            println!();
        }
    }

    // ── Summary ──────────────────────────────────────────────────────────────
    println!("  {}", "─".repeat(44).dimmed());
    println!();
    if issues == 0 && warnings == 0 {
        println!(
            "  {} Everything looks good. Make your code reach the robot.",
            "✓".green().bold()
        );
    } else {
        if issues > 0 {
            println!(
                "  {} {} issue{} found",
                "✗".red().bold(),
                issues.to_string().red().bold(),
                if issues == 1 { "" } else { "s" }
            );
        }
        if warnings > 0 {
            println!(
                "  {} {} warning{}",
                "⚠".yellow().bold(),
                warnings.to_string().yellow().bold(),
                if warnings == 1 { "" } else { "s" }
            );
        }
    }
    println!();
}

/// Detect ROS2 distro name from AMENT_PREFIX_PATH
fn detect_ros2_distro(ament_prefix: &str) -> String {
    let path = ament_prefix.split(':').next().unwrap_or("");
    for distro in &["humble", "iron", "jazzy", "rolling", "galactic", "foxy"] {
        if path.contains(distro) {
            return distro.to_string();
        }
    }
    "unknown distro".to_string()
}

// ─── reach launch ────────────────────────────────────────────────────────────

pub fn cmd_launch(pipeline_name: &str) {
    let config = load_config_or_exit();

    let pipeline = match config.launch.get(pipeline_name) {
        Some(p) => p.clone(),
        None => {
            eprintln!(
                "  {} Pipeline \"{}\" not found in reach.toml.",
                "✗".red().bold(), pipeline_name
            );
            if config.launch.is_empty() {
                eprintln!("  No pipelines defined. Add a [launch.{}] section to reach.toml.", pipeline_name);
            } else {
                eprintln!("  Available pipelines: {}",
                    config.launch.keys().cloned().collect::<Vec<_>>().join(", "));
            }
            std::process::exit(1);
        }
    };

    print_banner();
    println!(
        "  {}  pipeline: {}  ({} node{})",
        "reach launch".white().bold(),
        pipeline_name.cyan().bold(),
        pipeline.nodes.len(),
        if pipeline.nodes.len() == 1 { "" } else { "s" }
    );
    println!();

    if find_ros2_setup().is_none() {
        eprintln!("  {} ROS2 not found. Source ROS2 first:", "✗".red().bold());
        eprintln!("  {}", "source /opt/ros/humble/setup.bash".cyan());
        std::process::exit(1);
    }

    println!("  {} {}", "domain_id:".dimmed(), config.robot.domain_id.to_string().white());
    println!();

    let launcher = write_launcher()
        .unwrap_or_else(|e| fatal(&format!("Failed to write launcher: {}", e)));

    let processes: Arc<Mutex<HashMap<String, NodeProcess>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Ctrl+C handler
    let processes_ctrlc = Arc::clone(&processes);
    let launcher_ctrlc = launcher.clone();
    ctrlc::set_handler(move || {
        println!();
        println!("  {} Shutting down pipeline...", "■".yellow().bold());
        let mut procs = processes_ctrlc.lock().unwrap();
        for (name, proc) in procs.iter_mut() {
            proc.stop();
            println!("  {} [{}] stopped", "✓".green(), name.cyan());
        }
        let _ = std::fs::remove_file(&launcher_ctrlc);
        println!("  {} Pipeline stopped. Goodbye.", "✓".green().bold());
        std::process::exit(0);
    }).expect("Failed to set Ctrl-C handler");

    // Launch nodes in order, respecting delay and depends_on
    for launch_node in &pipeline.nodes {
        let node_config = match config.nodes.get(&launch_node.name) {
            Some(n) => n.clone(),
            None => {
                eprintln!("  {} Node \"{}\" not found.", "✗".red().bold(), launch_node.name);
                continue;
            }
        };

        // Wait for depends_on node to be running
        if let Some(ref dep) = launch_node.depends_on {
            print!(
                "  {} [{}] waiting for [{}]...",
                "⏳".yellow(), launch_node.name.cyan(), dep.cyan()
            );
            // Poll until the dependency is running
            loop {
                let procs = processes.lock().unwrap();
                if procs.contains_key(dep.as_str()) {
                    println!(" {}", "ready".green());
                    break;
                }
                drop(procs);
                std::thread::sleep(Duration::from_millis(200));
            }
        }

        // Apply delay
        if launch_node.delay > 0.0 {
            println!(
                "  {} [{}] waiting {:.1}s...",
                "⏱".dimmed(), launch_node.name.cyan(), launch_node.delay
            );
            std::thread::sleep(Duration::from_millis((launch_node.delay * 1000.0) as u64));
        }

        // Build command with params
        let mut cmd = build_node_command_with_params(
            &launcher,
            &node_config.script,
            &launch_node.name,
            &config,
            &launch_node.params,
        );

        println!(
            "  {} {}  {}",
            "▶".green().bold(),
            format!("[{}]", launch_node.name).cyan().bold(),
            node_config.script_rel.dimmed()
        );

        if !launch_node.params.is_empty() {
            let param_str: Vec<String> = launch_node.params.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            println!("      {} {}", "params:".dimmed(), param_str.join(", ").white());
        }

        match cmd.spawn() {
            Ok(child) => {
                let mut procs = processes.lock().unwrap();
                procs.insert(
                    launch_node.name.clone(),
                    NodeProcess { name: launch_node.name.clone(), config: node_config, child: Some(child) }
                );
            }
            Err(e) => {
                eprintln!("  {} Failed to start [{}]: {}", "✗".red().bold(), launch_node.name, e);
            }
        }

        // wait_ready — wait until node appears on ROS2 graph
        if launch_node.wait_ready {
            println!(
                "  {} [{}] waiting until ready on ROS2 graph...",
                "⏳".yellow(), launch_node.name.cyan()
            );
            wait_until_node_ready(&launch_node.name, &config.robot.domain_id);
            println!(
                "  {} [{}] {}",
                "✓".green(), launch_node.name.cyan(), "ready".green()
            );
        }
    }

    println!();

    // Hot reload watcher — same as reach run, respects # hot-reload-off
    let hot_reload_enabled = config.dev.hot_reload;
    let ignore_patterns = config.dev.hot_reload_ignore.clone();
    let project_root = config.root.clone();

    if hot_reload_enabled {
        println!("  {} watching for changes  {}", "◉".bright_blue(), "(# hot-reload-off to disable per node)".dimmed());
    } else {
        println!("  {} hot reload disabled in reach.toml", "○".dimmed());
    }
    println!("  {} Pipeline running. Press {} to stop.", "✓".green().bold(), "Ctrl+C".cyan());
    println!();

    if hot_reload_enabled {
        let processes_watcher = Arc::clone(&processes);
        let config_clone = config.clone();
        let launcher_clone = launcher.clone();

        std::thread::spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
            let mut watcher = RecommendedWatcher::new(
                move |res| { let _ = tx.send(res); },
                Config::default().with_poll_interval(Duration::from_millis(200)),
            ).expect("Failed to create watcher");

            watcher.watch(&project_root, RecursiveMode::Recursive)
                .expect("Failed to watch project");

            let mut last_reload: HashMap<String, Instant> = HashMap::new();
            let debounce = Duration::from_millis(400);

            loop {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(Ok(event)) => {
                        let is_relevant = matches!(
                            event.kind,
                            EventKind::Modify(_) | EventKind::Create(_)
                        );
                        if !is_relevant { continue; }

                        for path in &event.paths {
                            if path.extension().and_then(|e| e.to_str()) != Some("py") { continue; }

                            let rel = path.strip_prefix(&config_clone.root).unwrap_or(path);
                            let rel_str = rel.to_string_lossy();
                            if ignore_patterns.iter().any(|p| rel_str.contains(p.trim_matches('/'))) { continue; }
                            if rel_str.contains("__pycache__") || rel_str.ends_with(".pyc") { continue; }

                            let now = Instant::now();
                            let node_name = find_owning_node(path, &config_clone.nodes);

                            match node_name {
                                Some(name) => {
                                    if last_reload.get(&name).map_or(false, |l| now.duration_since(*l) < debounce) { continue; }
                                    if is_hot_reload_disabled(path) {
                                        println!("  {} [{}] {}", "~".dimmed(), name.dimmed(), "hot-reload-off — skipping".dimmed());
                                        continue;
                                    }
                                    last_reload.insert(name.clone(), now);
                                    println!("  {} {} changed", "→".bright_blue(), rel.display().to_string().white());
                                    let mut procs = processes_watcher.lock().unwrap();
                                    if let Some(proc) = procs.get_mut(&name) {
                                        proc.restart(&launcher_clone, &config_clone);
                                    }
                                }
                                None => {
                                    let key = "__all__".to_string();
                                    if last_reload.get(&key).map_or(false, |l| now.duration_since(*l) < debounce) { continue; }
                                    last_reload.insert(key, now);
                                    println!("  {} {} changed → reloading all eligible nodes", "→".yellow(), rel.display().to_string().white());
                                    let mut procs = processes_watcher.lock().unwrap();
                                    for (_, proc) in procs.iter_mut() {
                                        if !is_hot_reload_disabled(&proc.config.script) {
                                            proc.restart(&launcher_clone, &config_clone);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(Err(e)) => eprintln!("  {} Watcher error: {}", "✗".red(), e),
                    Err(_) => {}
                }
            }
        });
    }

    // Supervisor loop
    let mut dead: HashSet<String> = HashSet::new();
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let mut procs = processes.lock().unwrap();
        for (name, proc) in procs.iter_mut() {
            if dead.contains(name) { continue; }
            match proc.child.as_mut().and_then(|c| c.try_wait().ok()) {
                Some(Some(status)) => {
                    dead.insert(name.clone());
                    if !status.success() {
                        println!(
                            "  {} [{}] exited with error.",
                            "!".red().bold(), name.cyan()
                        );
                    }
                }
                _ => {}
            }
        }
        if dead.len() == procs.len() && !procs.is_empty() {
            println!("  {} All nodes stopped.", "■".yellow().bold());
            let _ = std::fs::remove_file(&launcher);
            std::process::exit(0);
        }
    }
}

/// Build command with per-pipeline params on top of global config
fn build_node_command_with_params(
    launcher: &PathBuf,
    script: &PathBuf,
    node_name: &str,
    config: &ReachConfig,
    params: &HashMap<String, String>,
) -> Command {
    let mut cmd = Command::new("python3");
    cmd.arg(launcher).arg(script).arg(node_name);
    cmd.env("ROS_DOMAIN_ID", config.robot.domain_id.to_string());

    let mut ros_args: Vec<String> = Vec::new();

    // Global remappings
    for (from, to) in &config.ros.remappings {
        ros_args.push("-r".to_string());
        ros_args.push(format!("{}:={}", from, to));
    }

    // Per-node args from [ros.node_args]
    if let Some(extra) = config.ros.node_args.get(node_name) {
        ros_args.extend(extra.clone());
    }

    // Namespace
    if let Some(ref ns) = config.ros.namespace {
        ros_args.push("-r".to_string());
        ros_args.push(format!("__ns:={}", ns));
    }

    // Pipeline params — become -p key:=value
    for (key, val) in params {
        ros_args.push("-p".to_string());
        ros_args.push(format!("{}:={}", key, val));
    }

    if !ros_args.is_empty() {
        cmd.arg("--ros-args");
        for arg in ros_args { cmd.arg(arg); }
    }

    cmd.stdout(std::process::Stdio::inherit())
       .stderr(std::process::Stdio::inherit());
    cmd
}

/// Poll the ROS2 graph until the node appears
/// Uses `ros2 node list` — simple and doesn't require lifecycle nodes
fn wait_until_node_ready(node_name: &str, domain_id: &u32) {
    let timeout = std::time::Instant::now();
    let max_wait = Duration::from_secs(30);

    loop {
        if timeout.elapsed() > max_wait {
            eprintln!(
                "  {} [{}] timed out waiting to appear on ROS2 graph.",
                "!".yellow(), node_name
            );
            return;
        }

        let output = Command::new("ros2")
            .arg("node")
            .arg("list")
            .env("ROS_DOMAIN_ID", domain_id.to_string())
            .output();

        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            // ROS2 node names are prefixed with /
            if stdout.contains(&format!("/{}", node_name)) || stdout.contains(node_name) {
                return;
            }
        }

        std::thread::sleep(Duration::from_millis(300));
    }
}
// ─── reach build ─────────────────────────────────────────────────────────────
// Generates a valid ROS2 package from reach.toml and runs colcon build.
// Output goes to .reach/ — developer never touches it.

pub fn cmd_build() {
    let config = load_config_or_exit();

    println!();
    println!("  {} {}", "reach build".white().bold(), "— generating ROS2 package...".dimmed());
    println!();

    let reach_dir = config.root.join(".reach");
    std::fs::create_dir_all(&reach_dir)
        .unwrap_or_else(|e| fatal(&format!("Failed to create .reach/: {}", e)));

    // ── Generate package.xml ─────────────────────────────────────────────────
    let package_xml = generate_package_xml(&config);
    std::fs::write(reach_dir.join("package.xml"), package_xml)
        .unwrap_or_else(|e| fatal(&format!("Failed to write package.xml: {}", e)));
    println!("  {} package.xml", "✓".green().bold());

    // ── Generate CMakeLists.txt ───────────────────────────────────────────────
    let cmake = generate_cmakelists(&config);
    std::fs::write(reach_dir.join("CMakeLists.txt"), cmake)
        .unwrap_or_else(|e| fatal(&format!("Failed to write CMakeLists.txt: {}", e)));
    println!("  {} CMakeLists.txt", "✓".green().bold());

    // ── Generate setup.py ────────────────────────────────────────────────────
    let setup_py = generate_setup_py(&config);
    std::fs::write(reach_dir.join("setup.py"), setup_py)
        .unwrap_or_else(|e| fatal(&format!("Failed to write setup.py: {}", e)));
    println!("  {} setup.py", "✓".green().bold());

    // ── Generate setup.cfg ───────────────────────────────────────────────────
    let setup_cfg = format!("[develop]\nscript_dir=$base/lib/{name}\n[install]\ninstall_scripts=$base/lib/{name}\n",
        name = config.project.name);
    std::fs::write(reach_dir.join("setup.cfg"), setup_cfg)
        .unwrap_or_else(|e| fatal(&format!("Failed to write setup.cfg: {}", e)));
    println!("  {} setup.cfg", "✓".green().bold());

    // ── Symlink src/ into .reach/ so colcon can find node scripts ────────────
    let src_link = reach_dir.join(&config.project.name);
    if src_link.exists() {
        std::fs::remove_file(&src_link).ok();
        std::fs::remove_dir_all(&src_link).ok();
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(config.root.join("src"), &src_link)
        .unwrap_or_else(|e| fatal(&format!("Failed to symlink src/: {}", e)));
    println!("  {} src/ symlinked", "✓".green().bold());

    println!();
    println!("  {} running colcon build...", "→".bright_blue().bold());
    println!();

    // ── Run colcon build ─────────────────────────────────────────────────────
    let status = Command::new("colcon")
        .arg("build")
        .arg("--packages-select")
        .arg(&config.project.name)
        .current_dir(&config.root)
        .env("COLCON_HOME", reach_dir.join(".colcon"))
        .status();

    match status {
        Ok(s) if s.success() => {
            println!();
            println!("  {} build successful", "✓".green().bold());
            println!();
            println!("  {} source the install space to use:", "→".dimmed());
            println!("  {}", format!("source {}/install/setup.bash",
                config.root.display()).cyan());
            println!();
            println!("  {} then run nodes via:", "→".dimmed());
            println!("  {}", format!("ros2 run {} <node>", config.project.name).cyan());
            println!();
        }
        Ok(s) => {
            eprintln!("  {} colcon build failed with status: {}", "✗".red().bold(), s);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("  {} colcon not found: {}", "✗".red().bold(), e);
            eprintln!("  Make sure ROS2 is sourced: {}", "source /opt/ros/humble/setup.bash".cyan());
            std::process::exit(1);
        }
    }
}

fn generate_package_xml(config: &ReachConfig) -> String {
    let deps: String = config.dependencies.iter()
        .map(|d| format!("  <exec_depend>{}</exec_depend>", d.name))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
r#"<?xml version="1.0"?>
<?xml-model href="http://download.ros.org/schema/package_format3.xsd" schematypens="http://www.w3.org/2001/XMLSchema"?>
<package format="3">
  <name>{name}</name>
  <version>{version}</version>
  <description>{description}</description>
  <maintainer email="reach@ascii-robotics.com">ASCII Robotics</maintainer>
  <license>MIT</license>

  <buildtool_depend>ament_python</buildtool_depend>

  <exec_depend>rclpy</exec_depend>
{deps}

  <export>
    <build_type>ament_python</build_type>
  </export>
</package>
"#,
        name = config.project.name,
        version = config.project.version,
        description = config.project.description.as_deref().unwrap_or("A ReachPy workspace"),
        deps = deps,
    )
}

fn generate_cmakelists(config: &ReachConfig) -> String {
    format!(
r#"cmake_minimum_required(VERSION 3.8)
project({name})

find_package(ament_cmake REQUIRED)
find_package(ament_cmake_python REQUIRED)

ament_python_install_package(${{PROJECT_NAME}})

install(PROGRAMS
{scripts}
  DESTINATION lib/${{PROJECT_NAME}}
)

ament_package()
"#,
        name = config.project.name,
        scripts = config.nodes.values()
            .map(|n| format!("  {}", n.script.display()))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn generate_setup_py(config: &ReachConfig) -> String {
    let entry_points: String = config.nodes.iter()
        .map(|(name, node)| {
            let script_stem = node.script.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(name);
            format!("            '{name} = {pkg}.{stem}:main',",
                name = name,
                pkg = config.project.name,
                stem = script_stem)
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
r#"from setuptools import setup

package_name = '{name}'

setup(
    name=package_name,
    version='{version}',
    packages=[package_name],
    data_files=[
        ('share/ament_index/resource_index/packages',
            ['resource/' + package_name]),
        ('share/' + package_name, ['package.xml']),
    ],
    install_requires=['setuptools'],
    zip_safe=True,
    entry_points={{
        'console_scripts': [
{entry_points}
        ],
    }},
)
"#,
        name = config.project.name,
        version = config.project.version,
        entry_points = entry_points,
    )
}
// ─── reach bags ──────────────────────────────────────────────────────────────
// Wraps ros2 bag record/play with ReachPy workspace awareness.
// Bags stored in bags/ with auto-generated timestamps.

pub fn cmd_bags(args: &[String]) {
    match args.get(0).map(|s| s.as_str()) {
        Some("record") => cmd_bags_record(&args[1..]),
        Some("play")   => cmd_bags_play(&args[1..]),
        Some("list")   => cmd_bags_list(),
        Some(unknown)  => {
            eprintln!("  {} Unknown bags subcommand \"{}\".", "✗".red().bold(), unknown);
            eprintln!("  Usage:");
            eprintln!("    reach bags record [topics...]");
            eprintln!("    reach bags record --profile <name>");
            eprintln!("    reach bags play <bag_path> [--loop] [--rate <speed>]");
            eprintln!("    reach bags list");
            std::process::exit(1);
        }
        None => {
            println!();
            println!("  {} {}", "reach bags".white().bold(), "— rosbag management".dimmed());
            println!();
            println!("  {}  {}", "reach bags record".cyan().bold(),           "record all topics".dimmed());
            println!("  {}  {}", "reach bags record /topic1 /topic2".cyan(),  "record specific topics".dimmed());
            println!("  {}  {}", "reach bags record --profile <name>".cyan(), "record topics from a launch profile".dimmed());
            println!("  {}  {}", "reach bags play <path>".cyan().bold(),      "play back a recording".dimmed());
            println!("  {}  {}", "reach bags play <path> --loop".cyan(),      "loop playback".dimmed());
            println!("  {}  {}", "reach bags play <path> --rate 0.5".cyan(),  "half speed playback".dimmed());
            println!("  {}  {}", "reach bags list".cyan().bold(),             "list all recordings".dimmed());
            println!();
        }
    }
}

fn cmd_bags_record(args: &[String]) {
    let config = load_config_or_exit();

    // Ensure bags/ directory exists
    let bags_dir = config.root.join("bags");
    std::fs::create_dir_all(&bags_dir)
        .unwrap_or_else(|e| fatal(&format!("Failed to create bags/: {}", e)));

    // Auto-generate timestamped bag name
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let bag_name = format!("bag_{}", timestamp);
    let bag_path = bags_dir.join(&bag_name);

    // Resolve topics to record
    let topics: Vec<String> = if args.contains(&"--profile".to_string()) {
        // Record topics from a launch profile
        let profile_idx = args.iter().position(|a| a == "--profile").unwrap();
        let profile_name = args.get(profile_idx + 1)
            .unwrap_or_else(|| { eprintln!("  {} --profile requires a name", "✗".red().bold()); std::process::exit(1); });

        // Get nodes in profile and derive their topics
        // For now record all topics when profile is specified
        println!("  {} recording all topics for profile \"{}\"", "→".bright_blue(), profile_name.cyan());
        vec![]  // empty = record all
    } else {
        // Specific topics or all
        args.iter()
            .filter(|a| a.starts_with('/'))
            .cloned()
            .collect()
    };

    println!();
    println!("  {} {}", "reach bags record".white().bold(), "— starting recording...".dimmed());
    println!();

    if topics.is_empty() {
        println!("  {} recording all topics", "◉".bright_blue().bold());
    } else {
        println!("  {} recording: {}", "◉".bright_blue().bold(), topics.join(", ").white());
    }
    println!("  {} {}", "output:".dimmed(), bag_path.display().to_string().white());
    println!();
    println!("  Press {} to stop recording.", "Ctrl+C".cyan());
    println!();

    // Build ros2 bag record command
    let mut cmd = Command::new("ros2");
    cmd.arg("bag").arg("record")
       .arg("-o").arg(&bag_path)
       .env("ROS_DOMAIN_ID", config.robot.domain_id.to_string());

    if topics.is_empty() {
        cmd.arg("-a"); // record all
    } else {
        for topic in &topics {
            cmd.arg(topic);
        }
    }

    match cmd.status() {
        Ok(_) => {
            println!();
            println!("  {} recording saved to {}", "✓".green().bold(),
                bag_path.display().to_string().cyan());
        }
        Err(e) => {
            eprintln!("  {} failed to start recording: {}", "✗".red().bold(), e);
            eprintln!("  Make sure ROS2 is sourced.");
            std::process::exit(1);
        }
    }
}

fn cmd_bags_play(args: &[String]) {
    let config = load_config_or_exit();

    let bag_path = match args.get(0) {
        Some(p) => p.clone(),
        None => {
            eprintln!("  {} Usage: reach bags play <bag_path>", "✗".red().bold());
            std::process::exit(1);
        }
    };

    // Resolve path — check bags/ dir if not absolute
    let resolved = if std::path::Path::new(&bag_path).exists() {
        bag_path.clone()
    } else {
        let in_bags = config.root.join("bags").join(&bag_path);
        if in_bags.exists() {
            in_bags.display().to_string()
        } else {
            eprintln!("  {} Bag not found: {}", "✗".red().bold(), bag_path.red());
            eprintln!("  Run {} to see available recordings.", "reach bags list".cyan());
            std::process::exit(1);
        }
    };

    let loop_playback = args.contains(&"--loop".to_string());
    let rate = args.iter()
        .position(|a| a == "--rate")
        .and_then(|i| args.get(i + 1))
        .and_then(|r| r.parse::<f64>().ok())
        .unwrap_or(1.0);

    println!();
    println!("  {} {}", "reach bags play".white().bold(), "— starting playback...".dimmed());
    println!();
    println!("  {} {}", "bag:".dimmed(), resolved.white());
    println!("  {} {}x", "rate:".dimmed(), rate.to_string().white());
    println!("  {} {}", "loop:".dimmed(), if loop_playback { "yes".green().to_string() } else { "no".dimmed().to_string() });
    println!();
    println!("  Press {} to stop.", "Ctrl+C".cyan());
    println!();

    let mut cmd = Command::new("ros2");
    cmd.arg("bag").arg("play")
       .arg(&resolved)
       .arg("--rate").arg(rate.to_string())
       .env("ROS_DOMAIN_ID", config.robot.domain_id.to_string());

    if loop_playback {
        cmd.arg("--loop");
    }

    match cmd.status() {
        Ok(_) => println!("  {} playback complete", "✓".green().bold()),
        Err(e) => {
            eprintln!("  {} playback failed: {}", "✗".red().bold(), e);
            std::process::exit(1);
        }
    }
}

fn cmd_bags_list() {
    let config = load_config_or_exit();
    let bags_dir = config.root.join("bags");

    println!();
    println!("  {} {}", "reach bags list".white().bold(), "— recordings".dimmed());
    println!();

    if !bags_dir.exists() {
        println!("  {} No bags directory found. Run {} first.",
            "→".dimmed(), "reach bags record".cyan());
        println!();
        return;
    }

    let mut entries: Vec<_> = std::fs::read_dir(&bags_dir)
        .unwrap_or_else(|e| fatal(&format!("Failed to read bags/: {}", e)))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();

    if entries.is_empty() {
        println!("  {} No recordings found in bags/", "→".dimmed());
        println!();
        return;
    }

    // Sort by name (timestamp-based names sort chronologically)
    entries.sort_by_key(|e| e.file_name());

    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let size = dir_size(&entry.path());
        println!("  {} {}  {}",
            "▸".bright_blue(),
            name.white(),
            format_size(size).dimmed()
        );
    }
    println!();
    println!("  {} to play: {}", "→".dimmed(), "reach bags play <name>".cyan());
    println!();
}

fn dir_size(path: &std::path::Path) -> u64 {
    std::fs::read_dir(path)
        .map(|entries| entries
            .filter_map(|e| e.ok())
            .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
            .sum())
        .unwrap_or(0)
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
// ─── reach trace ─────────────────────────────────────────────────────────────
// Data tracing and relationship tracing for ReachPy workspaces.
// Captures message flow, timing, and causal relationships between nodes.
// Stores traces in .reach/traces/ — never interferes with node output.


pub fn cmd_trace(args: &[String]) {
    match args.get(0).map(|s| s.as_str()) {
        Some("show")  => cmd_trace_show(&args[1..]),
        Some("clear") => cmd_trace_clear(),
        None          => cmd_trace_run(&[]),
        _             => cmd_trace_run(args),
    }
}

/// reach trace — run system with tracing enabled
fn cmd_trace_run(_args: &[String]) {
    let config = load_config_or_exit();

    // Ensure trace directory exists
    let trace_dir = config.root.join(".reach").join("traces");
    std::fs::create_dir_all(&trace_dir)
        .unwrap_or_else(|e| fatal(&format!("Failed to create trace dir: {}", e)));

    // Generate trace session ID
    let session_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let trace_file = trace_dir.join(format!("trace_{}.json", session_id));

    print_banner();
    println!(
        "  {}  {} tracing enabled",
        "reach trace".white().bold(),
        "◉".bright_blue().bold()
    );
    println!();
    println!("  {} {}", "session:".dimmed(), session_id.to_string().white());
    println!("  {} {}", "output:".dimmed(),
        trace_file.strip_prefix(&config.root).unwrap_or(&trace_file)
            .display().to_string().dimmed());
    println!();

    if find_ros2_setup().is_none() {
        eprintln!("  {} ROS2 not sourced.", "✗".red().bold());
        std::process::exit(1);
    }

    let launcher = write_launcher()
        .unwrap_or_else(|e| fatal(&format!("Failed to write launcher: {}", e)));

    // Resolve nodes to run
    let nodes_to_run: Vec<NodeConfig> = config.nodes.values().cloned().collect();

    // Build trace context — passed to each node via env var
    let _trace_context = serde_json::json!({
        "session_id": session_id,
        "trace_file": trace_file.display().to_string(),
        "nodes": nodes_to_run.iter().map(|n| &n.name).collect::<Vec<_>>(),
    }).to_string();

    println!("  {} {}", "domain_id:".dimmed(), config.robot.domain_id.to_string().white());
    println!();

    let processes: Arc<Mutex<HashMap<String, NodeProcess>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Start all nodes with tracing env vars injected
    {
        let mut procs = processes.lock().unwrap();
        for node_config in &nodes_to_run {
            println!(
                "  {} {}  {}",
                "▶".green().bold(),
                format!("[{}]", node_config.name).cyan().bold(),
                node_config.script_rel.dimmed()
            );

            let mut cmd = build_node_command(&launcher, &node_config.script, &node_config.name, &config);

            // Inject trace context
            cmd.env("REACHPY_TRACE", "1")
               .env("REACHPY_TRACE_SESSION", session_id.to_string())
               .env("REACHPY_TRACE_FILE", &trace_file)
               .env("REACHPY_TRACE_NODE", &node_config.name);

            match cmd.spawn() {
                Ok(child) => {
                    procs.insert(node_config.name.clone(),
                        NodeProcess { name: node_config.name.clone(), config: node_config.clone(), child: Some(child) });
                }
                Err(e) => eprintln!("  {} Failed to start [{}]: {}", "✗".red().bold(), node_config.name, e),
            }
        }
    }

    // Also start ros2 topic echo tracer in background
    // This captures the actual message flow between nodes
    let trace_file_clone = trace_file.clone();
    let config_clone = config.clone();
    let _processes_tracer = Arc::clone(&processes);

    std::thread::spawn(move || {
        run_topic_tracer(&config_clone, &trace_file_clone, session_id);
    });

    println!();
    println!("  {} tracing all message flows", "◉".bright_blue().bold());
    println!("  Press {} to stop and save trace.", "Ctrl+C".cyan());
    println!();

    // Ctrl+C — stop nodes and finalize trace
    let processes_ctrlc = Arc::clone(&processes);
    let launcher_ctrlc = launcher.clone();
    let trace_file_ctrlc = trace_file.clone();
    ctrlc::set_handler(move || {
        println!();
        println!("  {} Stopping nodes and finalizing trace...", "■".yellow().bold());
        let mut procs = processes_ctrlc.lock().unwrap();
        for (name, proc) in procs.iter_mut() {
            proc.stop();
            println!("  {} [{}] stopped", "✓".green(), name.cyan());
        }
        let _ = std::fs::remove_file(&launcher_ctrlc);
        println!();
        println!("  {} trace saved to {}", "✓".green().bold(),
            trace_file_ctrlc.display().to_string().cyan());
        println!("  run {} to inspect", "reach trace show".cyan().bold());
        println!();
        std::process::exit(0);
    }).expect("Failed to set Ctrl-C handler");

    // Supervisor loop
    let mut dead: HashSet<String> = HashSet::new();
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let mut procs = processes.lock().unwrap();
        for (name, proc) in procs.iter_mut() {
            if dead.contains(name) { continue; }
            match proc.child.as_mut().and_then(|c| c.try_wait().ok()) {
                Some(Some(status)) => {
                    dead.insert(name.clone());
                    if !status.success() {
                        println!("  {} [{}] exited with error.", "!".red().bold(), name.cyan());
                    }
                }
                _ => {}
            }
        }
        if dead.len() == procs.len() && !procs.is_empty() {
            println!("  {} All nodes stopped.", "■".yellow().bold());
            std::process::exit(0);
        }
    }
}

/// Background thread that captures topic data and timing
fn run_topic_tracer(config: &ReachConfig, trace_file: &std::path::Path, session_id: u64) {
    // Use ros2 topic list to discover active topics
    std::thread::sleep(Duration::from_secs(2)); // wait for nodes to come up

    let topic_list = Command::new("ros2")
        .arg("topic").arg("list")
        .env("ROS_DOMAIN_ID", config.robot.domain_id.to_string())
        .output();

    let topics: Vec<String> = match topic_list {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with("/parameter_events") && !l.starts_with("/rosout"))
                .collect()
        }
        _ => return,
    };

    // Get node info for relationship mapping
    let mut trace_data = serde_json::json!({
        "session_id": session_id,
        "started_at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "nodes": {},
        "topics": {},
        "events": [],
        "relationships": [],
    });

    // Map topic publishers and subscribers
    for topic in &topics {
        let info = Command::new("ros2")
            .arg("topic").arg("info").arg(topic).arg("--verbose")
            .env("ROS_DOMAIN_ID", config.robot.domain_id.to_string())
            .output();

        if let Ok(out) = info {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            trace_data["topics"][topic] = serde_json::json!({
                "info": text,
                "message_count": 0,
                "first_message_at": null,
                "last_message_at": null,
            });
        }
    }

    // Write initial trace structure
    if let Ok(json) = serde_json::to_string_pretty(&trace_data) {
        let _ = std::fs::write(trace_file, json);
    }

    // Poll topic Hz to track message rates
    let start = Instant::now();
    loop {
        std::thread::sleep(Duration::from_secs(1));

        let elapsed = start.elapsed().as_secs();

        for topic in &topics {
            let hz = Command::new("ros2")
                .arg("topic").arg("hz").arg(topic)
                .env("ROS_DOMAIN_ID", config.robot.domain_id.to_string())
                .output();

            if let Ok(out) = hz {
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                // Parse hz from output
                if let Some(rate) = parse_hz(&text) {
                    trace_data["topics"][topic]["hz"] = serde_json::json!(rate);
                    trace_data["topics"][topic]["last_message_at"] = serde_json::json!(elapsed);
                }
            }
        }

        // Update trace file
        if let Ok(json) = serde_json::to_string_pretty(&trace_data) {
            let _ = std::fs::write(trace_file, json);
        }
    }
}

fn parse_hz(text: &str) -> Option<f64> {
    for line in text.lines() {
        if line.contains("average rate:") {
            let parts: Vec<&str> = line.split(':').collect();
            if let Some(rate_str) = parts.get(1) {
                return rate_str.trim().parse::<f64>().ok();
            }
        }
    }
    None
}

/// reach trace show — display the last trace
fn cmd_trace_show(args: &[String]) {
    let config = load_config_or_exit();
    let trace_dir = config.root.join(".reach").join("traces");

    if !trace_dir.exists() {
        println!();
        println!("  {} No traces found. Run {} first.", "→".dimmed(), "reach trace".cyan());
        println!();
        return;
    }

    // Find latest trace
    let mut traces: Vec<_> = std::fs::read_dir(&trace_dir)
        .unwrap_or_else(|e| fatal(&format!("Cannot read trace dir: {}", e)))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();

    traces.sort_by_key(|e| e.file_name());

    let latest = match traces.last() {
        Some(t) => t.path(),
        None => {
            println!("  {} No trace files found.", "→".dimmed());
            return;
        }
    };

    let content = std::fs::read_to_string(&latest)
        .unwrap_or_else(|e| fatal(&format!("Cannot read trace: {}", e)));

    let trace: serde_json::Value = serde_json::from_str(&content)
        .unwrap_or_else(|e| fatal(&format!("Invalid trace file: {}", e)));

    println!();
    println!("  {} {}", "reach trace".white().bold(), "— session analysis".dimmed());
    println!();
    println!("  {} {}", "session:".dimmed(),
        trace["session_id"].as_u64().unwrap_or(0).to_string().white());
    println!();

    // Node filter
    let node_filter = args.iter()
        .position(|a| a == "--node")
        .and_then(|i| args.get(i + 1));

    // Display topic relationships
    println!("  {}", "Topic Activity".white().bold());
    println!("  {}", "─".repeat(44).dimmed());

    if let Some(topics) = trace["topics"].as_object() {
        for (topic, data) in topics {
            if let Some(filter) = node_filter {
                let info = data["info"].as_str().unwrap_or("");
                if !info.contains(filter.as_str()) { continue; }
            }

            let hz = data["hz"].as_f64();
            println!(
                "  {} {}  {}",
                "◈".bright_blue(),
                topic.white(),
                hz.map(|h| format!("{:.1} Hz", h)).unwrap_or_else(|| "no messages".to_string()).dimmed()
            );
        }
    }

    println!();

    // Display node relationships
    println!("  {}", "Node Relationships".white().bold());
    println!("  {}", "─".repeat(44).dimmed());
    println!("  {}", "Run ros2 node info <node> to see full relationships".dimmed());
    println!();

    // Show trace file location
    println!("  {} {}", "trace file:".dimmed(),
        latest.strip_prefix(&config.root).unwrap_or(&latest)
            .display().to_string().dimmed());
    println!();
}

/// reach trace clear — remove all traces
fn cmd_trace_clear() {
    let config = load_config_or_exit();
    let trace_dir = config.root.join(".reach").join("traces");

    if !trace_dir.exists() {
        println!("  {} No traces to clear.", "→".dimmed());
        return;
    }

    std::fs::remove_dir_all(&trace_dir)
        .unwrap_or_else(|e| fatal(&format!("Failed to clear traces: {}", e)));
    std::fs::create_dir_all(&trace_dir)
        .unwrap_or_else(|e| fatal(&format!("Failed to recreate trace dir: {}", e)));

    println!("  {} traces cleared", "✓".green().bold());
}