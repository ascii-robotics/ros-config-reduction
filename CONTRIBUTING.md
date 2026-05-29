# Contributing to ReachPy

First off — thank you. ReachPy is built on the belief that robotics development deserves better tooling, and every contribution moves that forward.

---

## Before you start

ReachPy is early stage. The CLI is v1.0.0. The Python framework doesn't exist yet. That means there's a lot of greenfield work available, but it also means things will move fast and designs will evolve.

If you're planning something significant — a new command, a change to `reach.toml` structure, a new crate — open an issue first and discuss it. Not to gatekeep, but to make sure your work doesn't conflict with something already in progress.

For small things — bug fixes, docs improvements, test coverage — just open a PR.

---

## Project structure

```
ros-config-reduction/
├── Cargo.toml                       ← workspace root
└── crates/
    ├── reachpy-config/              ← config library
    │   ├── Cargo.toml
    │   └── src/lib.rs               ← reach.toml parser and validator
    └── reach-cli/                   ← the reach binary
        ├── Cargo.toml
        ├── python/
        │   └── launcher.py          ← embedded into binary at compile time
        └── src/main.rs              ← all CLI commands
```

**reachpy-config** is the library every other component depends on. It parses, validates, and resolves `reach.toml` into typed Rust structs. If you're changing how `reach.toml` works — new fields, new sections, new validation rules — this is where that lives.

**reach-cli** is the binary. It depends on `reachpy-config` and implements all the commands — `reach create`, `reach run`, `reach launch`, `reach config`, `reach doctor`. `launcher.py` is embedded into the binary at compile time via `include_str!` so the binary is fully self-contained.

---

## Setting up

### Prerequisites

- [Rust](https://rustup.rs/) 1.75 or newer
- ROS2 Humble or newer (for testing `reach run` and `reach launch`)
- Python 3.10+

### Build

```bash
git clone https://github.com/ascii-robotics/ros-config-reduction.git
cd ros-config-reduction
cargo build
```

For a release build:

```bash
cargo build --release
```

The binary lands at `target/release/reach`. To use it system-wide:

```bash
sudo cp target/release/reach /usr/local/bin/reach
```

### Run tests

```bash
cargo test -p reachpy-config
```

Tests live in `crates/reachpy-config/src/lib.rs` under `#[cfg(test)]`. They test config parsing, validation, error messages, defaults, and pipeline resolution — all without needing ROS2.

---

## Making changes

### Adding a new `reach.toml` field

1. Add the raw field to the appropriate `Raw*` struct in `lib.rs` — make it `Option<T>` so missing fields give good errors
2. Add the resolved field to the corresponding resolved struct
3. Handle it in the `resolve()` function — validate, apply defaults, resolve paths if needed
4. Add a test

### Adding a new CLI command

1. Add `cmd_yourcommand()` function in `main.rs`
2. Wire it into the `match` in `main()`
3. Add it to `print_help()`
4. If it needs a new config field, update `reachpy-config` first

### Changing `launcher.py`

`launcher.py` lives at `crates/reach-cli/python/launcher.py`. It's embedded into the binary via:

```rust
const LAUNCHER_PY: &str = include_str!("../python/launcher.py");
```

Changes to `launcher.py` take effect after `cargo build`. No separate step needed.

---

## Testing reach run and reach launch

These commands need ROS2. To test:

```bash
source /opt/ros/humble/setup.bash
reach create test-workspace
cd test-workspace
reach run
```

For pipeline testing add `[launch.test]` to `reach.toml` and run `reach launch test`.

---

## Code style

- Rust: standard `rustfmt` formatting. Run `cargo fmt` before committing.
- Python: keep `launcher.py` minimal. It's an invisible shim, not a framework.
- Error messages: always actionable. Don't just say what went wrong — say what to do about it.
- CLI output: use the established color conventions — `✓` green for success, `✗` red for errors, `⚠` yellow for warnings, `→` blue for info.

---

## Opening a PR

- Keep PRs focused — one thing per PR
- Update tests if you're changing config parsing behavior
- Update the README if you're adding or changing a command
- PRs against `main` are fine — we don't have a separate dev branch yet

---

## Roadmap

Things we know we want to build, roughly in order:

- `reach build` — production bundler
- PyPI installable binary wrapper
- ReachPy Python framework (new repo) — `@node`, `Topic[T]`, `Message` types
- Custom message definitions without colcon
- ML model deployment integration
- Multi-platform binary releases (Linux ARM, macOS)

If you want to work on any of these open an issue so we can coordinate.

---

## Questions

Open an issue or reach out via [ascii-robotics.com](https://ascii-robotics.com).

Built at ASCII Robotics