<div align="center">

```
██████╗ ███████╗ █████╗  ██████╗██╗  ██╗
██╔══██╗██╔════╝██╔══██╗██╔════╝██║  ██║
██████╔╝█████╗  ███████║██║     ███████║
██╔══██╗██╔══╝  ██╔══██║██║     ██╔══██║
██║  ██║███████╗██║  ██║╚██████╗██║  ██║
╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝ ╚═════╝╚═╝  ╚═╝
```

**Make your code reach the robot.**

A Python-native robotics framework built on ROS2.  
ReachPy does to robotics development what Next.js did to web development.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)](https://www.rust-lang.org/)
[![ROS2](https://img.shields.io/badge/transport-ROS2-22314E.svg)](https://ros.org/)
[![Version](https://img.shields.io/badge/version-v1.1.0-blue.svg)](https://github.com/ascii-robotics/ros-config-reduction/releases)

</div>

---

## The problem

ROS workspaces are organized and messy at the same time. Every project requires:

- `CMakeLists.txt` — build configuration
- `package.xml` — package metadata and dependencies
- `setup.py` — Python package installation
- XML launch files — node orchestration
- `source devel/setup.bash` — invisible state that silently breaks things
- `colcon build --symlink-install` — every time you change a single line

That is four files saying the same things in four different formats before you have written a single line of robot logic. ReachPy replaces all of it with one file.

---

## Installation

### Prerequisites

- [Rust](https://rustup.rs/) 1.75+
- ROS2 Humble or newer
- Python 3.10+

### Build from source

```bash
git clone https://github.com/ascii-robotics/ros-config-reduction.git
cd ros-config-reduction
cargo build --release
sudo cp target/release/reach /usr/local/bin/reach
```

---

## reach.toml

One file. Replaces everything.

```toml
[project]
name = "my-robot"
version = "0.1.0"
python = "3.11"
description = "A ReachPy workspace"

[robot]
platform = "ur5"
transport = "ros2"
domain_id = 0

[nodes]
detector    = "src/detector.py"
controller  = "src/controller.py"
logger      = "src/logger.py"

[launch.default]
nodes = [
    { name = "detector",   delay = 0.0, wait_ready = true, params = { model = "yolo.pt" } },
    { name = "controller", delay = 2.0, depends_on = "detector" },
]

[launch.prod]
nodes = [
    { name = "detector",   delay = 0.0, wait_ready = true, params = { model = "yolo.pt", threshold = "0.9" } },
    { name = "controller", delay = 2.0, depends_on = "detector" },
    { name = "logger",     delay = 0.5 },
]

[launch.camera_only]
nodes = [
    { name = "detector", params = { model = "yolo.pt" } },
]

[dependencies]
opencv = "4.8"
torch  = "2.0"

[dev]
hot_reload = true
hot_reload_ignore = ["config/", "models/"]

# Advanced ROS2 escape hatch
# [ros]
# namespace = "/robot1"
# [ros.remappings]
# "/camera/raw" = "/camera/compressed"
```

### What reach.toml replaces

| reach.toml | Replaces |
|---|---|
| `[project]` | `package.xml` metadata + `setup.py` |
| `[nodes]` | `CMakeLists.txt` targets + entry points |
| `[launch.*]` | XML launch files — entirely |
| `[dependencies]` | `package.xml` exec_depend entries |
| `[robot] domain_id` | `ROS_DOMAIN_ID` env var |
| `[ros.remappings]` | `<remap>` tags in XML launch files |

---

## Writing nodes for ReachPy

Your nodes are plain Python files. The only requirement is exposing `__reachpy_node__` so the launcher can spin them:

```python
# src/detector.py
# hot-reload-off  ← add this to disable hot reload for this node

import rclpy
from rclpy.node import Node
from sensor_msgs.msg import Image
from std_msgs.msg import String

class DetectorNode(Node):
    def __init__(self):
        super().__init__('detector')
        self.sub = self.create_subscription(Image, '/camera/image_raw', self.callback, 10)
        self.pub = self.create_publisher(String, '/detections', 10)
        self.timer = self.create_timer(0.033, self.detect)

    def callback(self, msg):
        self.latest = msg

    def detect(self):
        # your logic here
        pass

# ReachPy picks this up and spins it
node = DetectorNode()
__reachpy_node__ = node
```

No `rclpy.init()`. No `rclpy.spin()`. No `rclpy.shutdown()`. ReachPy handles all of that.

---

## Commands

### `reach create <name>`

Scaffold a new workspace.

```bash
reach create my-robot
cd my-robot
```

```
my-robot/
├── reach.toml        ← your entire config lives here
├── src/
│   └── example.py   ← example node
├── config/           ← robot config (not hot-reloaded)
├── models/           ← ML models (not hot-reloaded)
└── bags/             ← rosbag recordings
```

---

### `reach run`

Run nodes with hot reload. Save a file, the node updates live. No rebuild. No restart.

```bash
reach run                     # runs [launch.default], or all nodes
reach run talker              # run a single node by name
reach run talker listener     # run specific nodes together
reach run camera_only         # run a named launch pipeline
```

Hot reload is always on. To disable it for a specific node, add this as the **first line** of that file:

```python
# hot-reload-off
```

ReachPy reads this before deciding whether to reload. The rest of your code is untouched.

---

### `reach launch <pipeline>`

Run a named launch pipeline with ordering, delays, dependencies, and per-node parameters. Hot reload enabled.

```bash
reach launch default
reach launch prod
reach launch camera_only
```

Pipeline node options:

| option | description |
|---|---|
| `name` | node name from `[nodes]` — required |
| `delay` | seconds to wait before starting this node |
| `depends_on` | wait for another node to be running first |
| `wait_ready` | block until node appears on ROS2 graph |
| `params` | ROS2 params passed as `--ros-args -p key:=value` |

---

### `reach build`

Generate a valid ROS2 package from `reach.toml` and run `colcon build`. Run this every time you want your workspace to be a proper ROS2 package — discoverable via `ros2 run`, dependable by other packages.

```bash
reach build
```

Generates `CMakeLists.txt`, `package.xml`, `setup.py` invisibly in `.reach/`. You never touch them. After build:

```bash
source .reach/install/setup.bash
ros2 run my-robot detector    # works
```

---

### `reach bags`

Rosbag management with workspace awareness. Bags stored in `bags/` with auto-generated timestamps.

```bash
# Recording
reach bags record                     # record all topics
reach bags record /camera /chatter    # record specific topics
reach bags record --profile default   # record topics from a launch profile

# Playback
reach bags play bags/bag_1234567890   # play back a recording
reach bags play bags/bag_1234567890 --loop          # loop
reach bags play bags/bag_1234567890 --rate 0.5      # half speed

# Management
reach bags list                       # list all recordings with sizes
```

---

### `reach trace`

Data tracing and relationship tracing. Captures message flow, timing, and node relationships while your system runs. Stored in `.reach/traces/`.

```bash
reach trace                    # run system with tracing enabled
reach trace show               # show last trace session
reach trace show --node talker # filter by node
reach trace clear              # remove all traces
```

Example output:

```
  reach trace — session analysis

  session: 1736934721

  Topic Activity
  ────────────────────────────────────────────
  ◈ /camera/image_raw    30.0 Hz
  ◈ /vision/detections   29.8 Hz
  ◈ /robot/cmd_vel       10.0 Hz

  Node Relationships
  ────────────────────────────────────────────
  [camera] → /camera/image_raw → [detector]
  [detector] → /vision/detections → [controller]
  [controller] → /robot/cmd_vel → robot
```

---

### `reach config`

Validate and display your resolved `reach.toml`.

```bash
reach config
```

```
  ✓ reach.toml resolved successfully

  ────────────────────────────────────────────
  Project:     my-robot v0.1.0
  Python:      3.11
  Platform:    ur5
  Transport:   ros2
  Nodes:       3
    - detector   → src/detector.py
    - controller → src/controller.py
    - logger     → src/logger.py
  Pipelines:
    - default → [detector, controller]
    - prod    → [detector, controller, logger]
  Hot reload:  true
  ────────────────────────────────────────────
```

---

### `reach doctor`

Diagnose your entire workspace before you try to run anything.

```bash
reach doctor
```

Checks:
- ROS2 sourced and distro detected
- rclpy importable
- Python version matches `reach.toml`
- `reach.toml` valid
- All node scripts exist and have no syntax errors
- `# hot-reload-off` flags detected
- Launch pipeline references are valid
- Dependencies installed

---

## How it works

```
reach run / reach launch
    → reads reach.toml (Rust)
    → resolves launch profile or pipeline
    → writes embedded launcher.py to /tmp (Rust)
    → spawns python3 launcher.py <script> <name> --ros-args ... (Rust)
    → injects ROS_DOMAIN_ID, remappings, params (Rust)
    → file watcher detects .py changes → restarts affected node (Rust)
    → launcher.py bootstraps rclpy → imports script → spins node (Python)

reach build
    → reads reach.toml
    → generates CMakeLists.txt, package.xml, setup.py in .reach/
    → runs colcon build
    → workspace becomes a valid ROS2 package

reach trace
    → runs nodes with REACHPY_TRACE env vars injected
    → background thread captures topic Hz and node relationships via ros2 CLI
    → stores structured JSON in .reach/traces/
    → reach trace show renders the session
```

The `reach` binary is self-contained. `launcher.py` is embedded at compile time via `include_str!` — no loose files, no installation steps beyond copying the binary.

---

## Project structure

```
ros-config-reduction/
├── Cargo.toml                       ← workspace root
└── crates/
    ├── reachpy-config/              ← config library
    │   ├── Cargo.toml
    │   └── src/lib.rs
    └── reach-cli/                   ← the reach binary
        ├── Cargo.toml
        ├── python/
        │   └── launcher.py          ← embedded into binary at compile time
        └── src/main.rs
```

---

## Roadmap

- [x] `reach create` — workspace scaffolding
- [x] `reach config` — config validation and display
- [x] `reach run` — flat node launcher with hot reload
- [x] `reach launch` — ordered pipeline launcher with hot reload
- [x] `reach doctor` — workspace diagnostics
- [x] `reach build` — ROS2 package generation
- [x] `reach bags` — rosbag recording and playback
- [x] `reach trace` — data and relationship tracing
- [x] `# hot-reload-off` — per-node opt out
- [ ] ReachPy Python framework — `@node`, `Topic[T]`, `Message` types
- [ ] Zenoh transport layer — ROS2-free operation
- [ ] `reach deploy` — deploy to robot fleet

---

## Built by

[ASCII Robotics](https://ascii-robotics.com) — Fukuoka, Japan  
Kyushu Institute of Technology

---

*"If it doesn't directly express robot behavior, ReachPy handles it invisibly."*
