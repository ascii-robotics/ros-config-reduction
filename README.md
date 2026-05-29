# Reach

**Make your code reach the robot.**

Reach (ReachPy) is a lightweight toolchain for ROS 2 Python robotics projects. It replaces the usual scatter of `CMakeLists.txt`, `package.xml`, `setup.py`, and XML launch files with a single **`reach.toml`** and a small CLI that runs your nodes with hot reload.

## Why Reach?

Traditional ROS 2 workspaces carry a lot of ceremony for what is often a handful of Python scripts. Reach keeps the same ROS 2 runtime (`rclpy`, topics, parameters) but moves configuration and orchestration into one place:

| Traditional stack | Reach |
|-------------------|-------|
| `package.xml` | `[project]` in `reach.toml` |
| `setup.py` / install rules | `[nodes]` paths |
| XML launch files | `[launch.<pipeline>]` |
| Manual `ros2 run` loops | `reach run` / `reach launch` |

You still source ROS 2 and write normal Python node scripts; Reach handles process management, remappings, and dev-time hot reload.

## Requirements

- **Rust** (2021 edition) — to build the `reach` CLI
- **Python 3.x** — node scripts
- **ROS 2** (Humble, Iron, or Jazzy) — sourced before running nodes, e.g.:

  ```bash
  source /opt/ros/humble/setup.bash
  ```

## Install

From the repository root:

```bash
cargo build --release
```

The binary is `target/release/reach`. Add it to your `PATH`, or install with:

```bash
cargo install --path crates/reach-cli
```

## Quick start

```bash
# Create a new workspace
reach create my-robot
cd my-robot

# Check your environment and config
reach doctor
reach config

# Run the default profile (with hot reload)
reach run
```

`reach create` scaffolds `reach.toml`, `src/`, `config/`, and `models/`.

## Commands

| Command | Description |
|---------|-------------|
| `reach` | Show help |
| `reach create <name>` | Create a new ReachPy workspace |
| `reach config` | Print resolved `reach.toml` |
| `reach doctor` | Diagnose ROS 2, Python, workspace, and nodes |
| `reach run [profile\|node...]` | Run nodes with hot reload |
| `reach launch <pipeline>` | Run a launch pipeline (ordered, with delays/deps) |
| `reach build` | *(coming soon)* |

### `reach run`

Starts one or more nodes and watches `.py` files for changes.

- **No args** — runs the `default` launch profile, or all nodes if no `default` profile is defined
- **One arg** — if it matches a launch profile name, runs that profile; if it matches a node name, runs that node
- **Multiple args** — treats each arg as a node name

```bash
reach run                  # default profile
reach run dev              # profile named "dev"
reach run detector         # single node
reach run detector tracker # multiple nodes
```

### `reach launch`

Runs a named pipeline from `reach.toml` **in order**, honoring:

- **`delay`** — wait N seconds before starting a node
- **`depends_on`** — wait until another node in the pipeline is running
- **`wait_ready`** — block until the node appears on the ROS 2 graph (`ros2 node list`)
- **`params`** — per-node ROS parameters (`--ros-args -p key:=value`)

Use `reach run` for everyday dev (parallel start + hot reload). Use `reach launch` when startup order and readiness gates matter.

## `reach.toml` reference

A minimal workspace:

```toml
[project]
name = "my-robot"
version = "0.1.0"
python = "3.11"

[robot]
platform = "generic"
transport = "ros2"
domain_id = 0

[nodes]
detector   = "src/detector.py"
controller = "src/controller.py"

[launch.default]
nodes = [
    { name = "detector" },
    { name = "controller", delay = 1.0, depends_on = "detector" },
]

[launch.prod]
nodes = [
    { name = "detector", wait_ready = true, params = { model = "yolo.pt" } },
    { name = "controller", delay = 2.0, depends_on = "detector" },
]

[dev]
hot_reload = true
hot_reload_ignore = ["config/", "models/"]

[ros]
remappings = { "/camera/image" = "/robot/camera/image" }
namespace = "/my_robot"
```

### Sections

| Section | Purpose |
|---------|---------|
| `[project]` | Workspace name, version, Python version |
| `[robot]` | Platform, transport (`ros2`), `domain_id` |
| `[nodes]` | Map of node name → script path (relative to workspace root) |
| `[launch.<name>]` | Named pipelines; each has a `nodes` array of step objects |
| `[dependencies]` | Python packages (checked by `reach doctor`) |
| `[dev]` | Hot reload toggle and ignore paths |
| `[ros]` | Global remappings, namespace, per-node args, parameters |

Config is discovered by walking up from the current directory until `reach.toml` is found.

## Writing nodes

Node scripts are plain Python. Reach embeds a launcher that:

1. Initializes `rclpy`
2. Imports your script as a module
3. Spins a `@node` if one is registered (future); otherwise runs your script’s top-level code

Example (`src/example.py`):

```python
import time

print("[example] Node started")
while True:
    time.sleep(1)
```

For ROS 2 nodes, use `rclpy` as usual inside the script.

## Hot reload

When `[dev] hot_reload = true` (default), Reach watches the workspace for `.py` changes and restarts the affected node(s).

- Add `# hot-reload-off` in the first 5 lines of a file to skip reload for that node
- Paths in `hot_reload_ignore` (e.g. `config/`, `models/`) are not watched
- Shared modules outside a node’s directory reload all eligible running nodes

Press **Ctrl+C** to stop all nodes cleanly.

## Workspace layout

```
my-robot/
├── reach.toml      # All project config
├── src/            # Node scripts
├── config/         # Robot config (ignored by hot reload)
├── models/         # ML weights (ignored by hot reload)
└── launch/         # Reserved for future use
```

## Repository structure

```
.
├── Cargo.toml              # Workspace root
├── crates/
│   ├── reach-cli/          # `reach` binary (create, run, launch, doctor)
│   │   └── python/
│   │       └── launcher.py # Embedded rclpy bootstrap (not user-facing)
│   └── reachpy-config/     # reach.toml parser and validation
└── README.md
```

## Development

```bash
# Build
cargo build

# Run tests (config crate)
cargo test -p reachpy-config

# Run CLI from workspace
cargo run -p reach-cli -- doctor
```

## Status

Reach is early-stage. `reach build` and deeper ROS package integration are planned. The config format and CLI are actively evolving.

## License

See repository defaults; confirm license file if present before redistribution.
