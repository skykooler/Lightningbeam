#!/usr/bin/env python3
"""
Transform all instrument preset JSON files to add a Compressor → Pan → Gain
output chain, plus Volume and Pan AutomationInput nodes wired via CV.

Only modifies presets that have a MidiInput node (i.e., MIDI instrument presets).
"""

import json
import sys
from pathlib import Path


def transform_preset(data: dict) -> dict | None:
    """Transform a preset. Returns modified dict or None if no change needed."""
    nodes = data.get("nodes", [])
    connections = data.get("connections", [])

    # Only modify presets with a MidiInput node
    if not any(n["node_type"] == "MidiInput" for n in nodes):
        return None

    # Skip if already transformed (has a Compressor node)
    if any(n["node_type"] == "Compressor" for n in nodes):
        print("  Already transformed, skipping.")
        return None

    output_node_id = data.get("output_node")
    if output_node_id is None:
        print("  No output_node, skipping.")
        return None

    # Find the connection going into the output node
    incoming = [c for c in connections if c["to_node"] == output_node_id]
    if len(incoming) != 1:
        print(f"  Expected 1 incoming connection to output_node, found {len(incoming)}, skipping.")
        return None

    conn = incoming[0]
    source_node = conn["from_node"]
    source_port = conn["from_port"]

    # Get AudioOutput node position — new chain starts where it was
    output_node_data = next((n for n in nodes if n["id"] == output_node_id), None)
    out_pos = output_node_data.get("position", [700.0, 150.0]) if output_node_data else [700.0, 150.0]
    if isinstance(out_pos, list):
        ox, oy = float(out_pos[0]), float(out_pos[1])
    else:
        ox, oy = 700.0, 150.0

    step = 230.0  # horizontal spacing between nodes

    # Compute new node IDs
    max_id = max(n["id"] for n in nodes)
    comp_id     = max_id + 1  # Compressor
    pan_id      = max_id + 2  # Pan
    gain_id     = max_id + 3  # Gain (volume)
    vol_id      = max_id + 4  # Volume AutomationInput
    pan_auto_id = max_id + 5  # Pan AutomationInput

    # Move the AudioOutput node to the right of the new chain
    if output_node_data is not None:
        output_node_data["position"] = [ox + step * 3, oy]

    # Remove the existing connection to output
    connections = [c for c in connections if not (c["to_node"] == output_node_id and c["from_node"] == source_node)]

    # New nodes — Compressor starts where AudioOutput was
    new_nodes = [
        {
            "id": comp_id,
            "node_type": "Compressor",
            "parameters": {"0": -18.0, "1": 4.0, "2": 5.0, "3": 50.0, "4": 3.0, "5": 3.0},
            "position": [ox, oy]
        },
        {
            "id": pan_id,
            "node_type": "Pan",
            "parameters": {"0": 0.0},
            "position": [ox + step, oy]
        },
        {
            "id": gain_id,
            "node_type": "Gain",
            "parameters": {"0": 1.0},
            "position": [ox + step * 2, oy]
        },
        {
            "id": vol_id,
            "node_type": "AutomationInput",
            "parameters": {"0": 0.0, "1": 2.0},
            "automation_display_name": "Volume",
            "automation_keyframes": [
                {
                    "time": 0.0,
                    "value": 1.0,
                    "interpolation": "linear",
                    "ease_out": [0.58, 1.0],
                    "ease_in": [0.42, 0.0]
                }
            ],
            "position": [ox + step, oy + 230.0]
        },
        {
            "id": pan_auto_id,
            "node_type": "AutomationInput",
            "parameters": {"0": -1.0, "1": 1.0},
            "automation_display_name": "Pan",
            "automation_keyframes": [
                {
                    "time": 0.0,
                    "value": 0.0,
                    "interpolation": "linear",
                    "ease_out": [0.58, 1.0],
                    "ease_in": [0.42, 0.0]
                }
            ],
            "position": [ox, oy + 230.0]
        },
    ]

    # New connections
    new_connections = [
        {"from_node": source_node,  "from_port": source_port, "to_node": comp_id,   "to_port": 0},
        {"from_node": comp_id,      "from_port": 0,           "to_node": pan_id,    "to_port": 0},
        {"from_node": pan_id,       "from_port": 0,           "to_node": gain_id,   "to_port": 0},
        {"from_node": gain_id,      "from_port": 0,           "to_node": output_node_id, "to_port": 0},
        {"from_node": vol_id,       "from_port": 0,           "to_node": gain_id,   "to_port": 1},
        {"from_node": pan_auto_id,  "from_port": 0,           "to_node": pan_id,    "to_port": 1},
    ]

    data["nodes"] = nodes + new_nodes
    data["connections"] = connections + new_connections
    return data


def main():
    instruments_dir = Path(__file__).parent.parent / "src" / "assets" / "instruments"
    if not instruments_dir.exists():
        print(f"Instruments directory not found: {instruments_dir}", file=sys.stderr)
        sys.exit(1)

    json_files = sorted(instruments_dir.rglob("*.json"))
    print(f"Found {len(json_files)} preset files")

    modified = 0
    for path in json_files:
        print(f"Processing: {path.relative_to(instruments_dir)}")
        with open(path) as f:
            data = json.load(f)

        result = transform_preset(data)
        if result is not None:
            with open(path, "w") as f:
                json.dump(result, f, indent=2)
            print(f"  -> Modified")
            modified += 1
        else:
            print(f"  -> Skipped")

    print(f"\nDone. Modified {modified}/{len(json_files)} presets.")


if __name__ == "__main__":
    main()
