#!/usr/bin/env python3
"""
ReachPy Node Launcher
---------------------
This file is embedded into the reach binary at compile time.
Users never see or touch this. It is the invisible bridge between
reach run and rclpy.

Usage (internal, called by reach run):
    python3 launcher.py <node_script> <node_name> [--ros-args ...]
"""

import sys
import os
import importlib.util
import traceback


def bootstrap(node_script: str, node_name: str):
    """
    Bootstrap rclpy and run a user's node script.
    The script is imported as a module — any top-level code runs,
    and if it defines a ReachPy @node, that gets spun up.
    If it's a plain Python script (pre-framework), it just runs.
    """
    try:
        import rclpy
    except ImportError:
        print(
            f"[ReachPy] ERROR: rclpy not found.\n"
            f"  Make sure ROS2 is sourced before running reach.\n"
            f"  Try: source /opt/ros/humble/setup.bash",
            file=sys.stderr
        )
        sys.exit(1)

    # Resolve absolute path
    script_path = os.path.abspath(node_script)
    if not os.path.exists(script_path):
        print(
            f"[ReachPy] ERROR: Node script not found: {script_path}",
            file=sys.stderr
        )
        sys.exit(1)

    # Add the script's directory to Python path
    # so relative imports within the workspace work
    script_dir = os.path.dirname(script_path)
    if script_dir not in sys.path:
        sys.path.insert(0, script_dir)

    # Also add the workspace root (two levels up from src/)
    workspace_root = os.path.dirname(script_dir)
    if workspace_root not in sys.path:
        sys.path.insert(0, workspace_root)

    print(f"[ReachPy] Starting node [{node_name}] from {node_script}")

    # Initialize rclpy
    rclpy.init(args=None)

    try:
        # Import the user's script as a module
        spec = importlib.util.spec_from_file_location(node_name, script_path)
        module = importlib.util.module_from_spec(spec)

        # Inject ReachPy context so the script knows its own name
        module.__reachpy_node_name__ = node_name

        spec.loader.exec_module(module)

        # If the module registered a ReachPy node, spin it
        # This will be populated by the @node decorator (future)
        if hasattr(module, "__reachpy_node__"):
            node = module.__reachpy_node__
            print(f"[ReachPy] [{node_name}] spinning...")
            try:
                rclpy.spin(node)
            finally:
                node.destroy_node()
        else:
            # Plain Python script — it ran its own loop already
            # Nothing to spin
            pass

    except KeyboardInterrupt:
        print(f"\n[ReachPy] [{node_name}] shutting down...")
    except Exception as e:
        print(
            f"[ReachPy] [{node_name}] crashed:\n"
            f"  {type(e).__name__}: {e}\n",
            file=sys.stderr
        )
        traceback.print_exc()
        sys.exit(1)
    finally:
        rclpy.shutdown()
        print(f"[ReachPy] [{node_name}] stopped.")


if __name__ == "__main__":
    if len(sys.argv) < 3:
        print(
            "Usage: launcher.py <node_script> <node_name> [--ros-args ...]",
            file=sys.stderr
        )
        sys.exit(1)

    node_script = sys.argv[1]
    node_name   = sys.argv[2]
    # Remaining args (--ros-args etc) are already in sys.argv
    # and will be picked up by rclpy.init()

    bootstrap(node_script, node_name)