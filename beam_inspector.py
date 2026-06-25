#!/usr/bin/env python3
"""
Lightningbeam .beam File Inspector

A command-line tool to inspect .beam project files.
"""

import argparse
import json
import sqlite3
import sys
import uuid as uuidlib
import zipfile
from datetime import datetime
from pathlib import Path
from typing import Any, Dict, List, Optional


# First 16 bytes of any SQLite 3 database.
SQLITE_MAGIC = b"SQLite format 3\x00"

# media.kind values (see BEAM_FILE_FORMAT.md §6.1).
MEDIA_KIND_NAMES = {
    0: "Audio", 1: "Video", 2: "Raster", 3: "ImageAsset",
    4: "Waveform", 5: "Thumbnail", 6: "RasterProxy",
}
# media.storage values (§6.2).
MEDIA_STORAGE_NAMES = {0: "Packed", 1: "Referenced"}


def _is_sqlite(path: Path) -> bool:
    """True if the file begins with the SQLite 3 header magic."""
    try:
        with open(path, "rb") as f:
            return f.read(16) == SQLITE_MAGIC
    except OSError:
        return False


def _human_size(n: int) -> str:
    f = float(n)
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if f < 1024 or unit == "TiB":
            return f"{f:.0f} {unit}" if unit == "B" else f"{f:.1f} {unit}"
        f /= 1024
    return f"{n} B"


class BeamInspector:
    """Inspector for .beam project files (SQLite container, or legacy ZIP)."""

    def __init__(self, beam_file: Path):
        self.beam_file = beam_file
        self.project_data: Optional[Dict[str, Any]] = None
        # SQLite (current) vs ZIP (legacy) container.
        self.is_sqlite = _is_sqlite(beam_file)
        # Populated for SQLite files by load():
        self.meta: Dict[str, str] = {}
        self.media: List[Dict[str, Any]] = []          # rows from the media table
        self.media_by_id: Dict[str, Dict[str, Any]] = {}  # uuid-string -> row

    def _connect(self) -> sqlite3.Connection:
        """Open the SQLite container read-only."""
        uri = self.beam_file.resolve().as_uri() + "?mode=ro"
        return sqlite3.connect(uri, uri=True)

    def load(self) -> bool:
        """Load and parse the .beam file (container-agnostic)."""
        try:
            if self.is_sqlite:
                return self._load_sqlite()
            return self._load_zip()
        except Exception as e:
            print(f"Error loading .beam file: {e}", file=sys.stderr)
            return False

    def _load_zip(self) -> bool:
        with zipfile.ZipFile(self.beam_file, 'r') as zip_ref:
            with zip_ref.open('project.json') as f:
                self.project_data = json.load(f)
        return True

    def _load_sqlite(self) -> bool:
        con = self._connect()
        try:
            cur = con.cursor()
            row = cur.execute("SELECT data FROM project_json WHERE id = 0").fetchone()
            if not row:
                raise ValueError("archive has no project.json row")
            self.project_data = json.loads(row[0])

            self.meta = {k: v for (k, v) in cur.execute("SELECT key, value FROM meta")}

            for (idb, kind, codec, storage, ext_path, total_len,
                 channels, sample_rate, width, height) in cur.execute(
                "SELECT id, kind, codec, storage, ext_path, total_len, "
                "channels, sample_rate, width, height FROM media"
            ):
                uid = str(uuidlib.UUID(bytes=bytes(idb)))
                info = {
                    "uuid": uid,
                    "kind": kind,
                    "codec": codec,
                    "storage": storage,
                    "ext_path": ext_path,
                    "total_len": total_len or 0,
                    "channels": channels,
                    "sample_rate": sample_rate,
                    "width": width,
                    "height": height,
                }
                self.media.append(info)
                self.media_by_id[uid] = info
        finally:
            con.close()
        return True

    def show_info(self):
        """Display basic project information."""
        if not self.project_data:
            return

        print("=" * 60)
        print("PROJECT INFORMATION")
        print("=" * 60)

        print(f"Container:     {'SQLite' if self.is_sqlite else 'Legacy ZIP'}")
        if self.is_sqlite:
            print(f"Schema Ver:    {self.meta.get('schema_version', 'Unknown')}")
        print(f"Version:       {self.project_data.get('version', 'Unknown')}")
        print(f"Created:       {self.project_data.get('created', 'Unknown')}")
        print(f"Modified:      {self.project_data.get('modified', 'Unknown')}")

        ui_state = self.project_data.get('ui_state', {})
        print(f"\nProject Name:  {ui_state.get('name', 'Unnamed')}")
        print(f"ID:            {ui_state.get('id', 'Unknown')}")
        print(f"Dimensions:    {ui_state.get('width', 0):.0f} x {ui_state.get('height', 0):.0f}")
        print(f"Framerate:     {ui_state.get('framerate', 0):.1f} fps")
        print(f"Duration:      {ui_state.get('duration', 0):.2f} seconds")

        bg = ui_state.get('background_color', {})
        print(f"Background:    rgba({bg.get('r', 0)}, {bg.get('g', 0)}, {bg.get('b', 0)}, {bg.get('a', 255)})")

        audio_backend = self.project_data.get('audio_backend', {})
        print(f"\nSample Rate:   {audio_backend.get('sample_rate', 0)} Hz")

    def show_clips(self):
        """Display clips information."""
        if not self.project_data:
            return

        ui_state = self.project_data.get('ui_state', {})

        vector_clips = ui_state.get('vector_clips', {})
        video_clips = ui_state.get('video_clips', {})
        audio_clips = ui_state.get('audio_clips', {})

        print("\n" + "=" * 60)
        print("CLIPS")
        print("=" * 60)

        print(f"Vector Clips:  {len(vector_clips)}")
        print(f"Video Clips:   {len(video_clips)}")
        print(f"Audio Clips:   {len(audio_clips)}")

        if vector_clips:
            print("\nVector Clips:")
            for clip_id, clip in vector_clips.items():
                print(f"  - {clip.get('name', 'Unnamed')} (ID: {clip_id[:8]}...)")

        if video_clips:
            print("\nVideo Clips:")
            for clip_id, clip in video_clips.items():
                print(f"  - {clip.get('name', 'Unnamed')} (ID: {clip_id[:8]}...)")

        if audio_clips:
            print("\nAudio Clips:")
            for clip_id, clip in audio_clips.items():
                print(f"  - {clip.get('name', 'Unnamed')} (ID: {clip_id[:8]}...)")

    def show_layers(self):
        """Display layer hierarchy."""
        if not self.project_data:
            return

        ui_state = self.project_data.get('ui_state', {})
        root = ui_state.get('root', {})

        print("\n" + "=" * 60)
        print("LAYER HIERARCHY")
        print("=" * 60)

        def print_layer(layer: Dict[str, Any], indent: int = 0):
            # Handle case where layer might not be a dictionary
            if not isinstance(layer, dict):
                prefix = "  " * indent
                print(f"{prefix}- ERROR: Layer is {type(layer).__name__}, not a dict: {repr(layer)[:50]}")
                return

            layer_type = layer.get('type', 'Unknown')
            layer_name = layer.get('name', 'Unnamed')
            layer_id = layer.get('id', 'Unknown')

            prefix = "  " * indent
            # Handle layer_id that might not be a string
            id_str = layer_id[:8] + "..." if isinstance(layer_id, str) and len(layer_id) > 8 else str(layer_id)
            print(f"{prefix}- [{layer_type}] {layer_name} (ID: {id_str})")

            # Recursively print children
            children = layer.get('children', [])
            for child in children:
                print_layer(child, indent + 1)

        print_layer(root)

    def show_tracks(self):
        """Display audio tracks information."""
        if not self.project_data:
            return

        audio_backend = self.project_data.get('audio_backend', {})
        project = audio_backend.get('project', {})
        tracks_dict = project.get('tracks', {})
        root_tracks = project.get('root_tracks', [])
        master_track = project.get('master_track', {})

        print("\n" + "=" * 60)
        print("AUDIO TRACKS")
        print("=" * 60)

        print(f"Total Tracks:  {len(tracks_dict)}")

        # Iterate through root_tracks in order
        for track_id in root_tracks:
            track_id_str = str(track_id)
            if track_id_str not in tracks_dict:
                print(f"\n  Track {track_id}: ERROR - Track ID not found in tracks dict")
                continue

            track_node = tracks_dict[track_id_str]

            # TrackNode is an enum with variants like {"Audio": {...}} or {"Midi": {...}}
            if not isinstance(track_node, dict):
                print(f"\n  Track {track_id}: ERROR - Track node is {type(track_node).__name__}, not a dict")
                continue

            # Extract the variant (Audio, Midi, or Group)
            if len(track_node) != 1:
                print(f"\n  Track {track_id}: ERROR - Track node has unexpected structure")
                continue

            track_type, track_data = list(track_node.items())[0]

            if not isinstance(track_data, dict):
                print(f"\n  Track {track_id}: ERROR - Track data is {type(track_data).__name__}, not a dict")
                continue

            track_name = track_data.get('name', f'Track {track_id}')
            muted = track_data.get('muted', False)
            solo = track_data.get('solo', False)
            volume = track_data.get('volume', 1.0)
            pan = track_data.get('pan', 0.0)

            status = []
            if muted:
                status.append('MUTED')
            if solo:
                status.append('SOLO')
            status_str = f" [{', '.join(status)}]" if status else ""

            print(f"\n  Track {track_id}: {track_name}{status_str}")
            print(f"    Type:      {track_type}")
            print(f"    Volume:    {volume:.2f}")
            print(f"    Pan:       {pan:.2f}")

            if track_type == "Midi":
                print(f"    Instrument: {track_data.get('instrument', 'Unknown')}")
                notes = track_data.get('notes', [])
                print(f"    Notes:     {len(notes)}")
                preset = track_data.get('instrument_graph_preset')
                if preset:
                    nodes = preset.get('nodes', [])
                    conns = preset.get('connections', [])
                    print(f"    Graph:     {len(nodes)} nodes, {len(conns)} connections")
                else:
                    print(f"    Graph:     (no preset saved)")
            elif track_type == "Audio":
                clips = track_data.get('clips', [])
                print(f"    Clips:     {len(clips)}")
                preset = track_data.get('effects_graph_preset')
                if preset:
                    nodes = preset.get('nodes', [])
                    conns = preset.get('connections', [])
                    print(f"    Graph:     {len(nodes)} nodes, {len(conns)} connections")
                else:
                    print(f"    Graph:     (no preset saved)")

        print(f"\nMaster Track:")
        print(f"  Volume:      {master_track.get('volume', 1.0):.2f}")

    def show_graphs(self):
        """Display detailed node graph information for all tracks."""
        if not self.project_data:
            return

        audio_backend = self.project_data.get('audio_backend', {})
        project = audio_backend.get('project', {})
        tracks_dict = project.get('tracks', {})

        print("\n" + "=" * 60)
        print("NODE GRAPHS")
        print("=" * 60)

        for track_id_str, track_node in tracks_dict.items():
            if not isinstance(track_node, dict) or len(track_node) != 1:
                continue

            track_type, track_data = list(track_node.items())[0]
            track_name = track_data.get('name', f'Track {track_id_str}')

            # Get the appropriate preset
            if track_type == "Audio":
                preset = track_data.get('effects_graph_preset')
                graph_label = "Effects Graph"
            elif track_type == "Midi":
                preset = track_data.get('instrument_graph_preset')
                graph_label = "Instrument Graph"
            else:
                continue

            print(f"\n  Track {track_id_str}: {track_name} ({track_type}) - {graph_label}")

            if not preset:
                print(f"    ** NO PRESET SAVED **")
                continue

            nodes = preset.get('nodes', [])
            connections = preset.get('connections', [])
            output_node = preset.get('output_node')
            midi_targets = preset.get('midi_targets', [])
            metadata = preset.get('metadata', {})

            if metadata:
                print(f"    Preset Name: {metadata.get('name', 'Unknown')}")

            print(f"    Nodes ({len(nodes)}):")
            for i, node in enumerate(nodes):
                node_type = node.get('node_type', '?')
                node_name = node.get('name', node_type)
                params = node.get('parameters', {})
                pos_x = node.get('position_x', 0)
                pos_y = node.get('position_y', 0)

                markers = []
                if output_node is not None and i == output_node:
                    markers.append("OUTPUT")
                if i in midi_targets:
                    markers.append("MIDI TARGET")
                marker_str = f" [{', '.join(markers)}]" if markers else ""

                print(f"      [{i}] {node_type}{marker_str}  (name={node_name}, pos={pos_x:.0f},{pos_y:.0f})")

                if params:
                    for param_name, param_val in params.items():
                        print(f"           {param_name} = {param_val}")

            print(f"    Connections ({len(connections)}):")
            for conn in connections:
                src = conn.get('from_node', '?')
                src_port = conn.get('from_output', '?')
                dst = conn.get('to_node', '?')
                dst_port = conn.get('to_input', '?')
                print(f"      [{src}]:{src_port} -> [{dst}]:{dst_port}")

        # Also show layer_to_track_map if present
        track_map = audio_backend.get('layer_to_track_map', {})
        if track_map:
            print(f"\n  Layer-to-Track Mapping ({len(track_map)} entries):")
            for layer_id, track_id in track_map.items():
                print(f"    {layer_id[:8]}... -> Track {track_id}")
        else:
            print(f"\n  Layer-to-Track Mapping: (none saved)")

    def show_audio_pool(self):
        """Display audio pool entries."""
        if not self.project_data:
            return

        audio_backend = self.project_data.get('audio_backend', {})
        pool_entries = audio_backend.get('audio_pool_entries', [])

        print("\n" + "=" * 60)
        print("AUDIO POOL")
        print("=" * 60)

        print(f"Total Entries: {len(pool_entries)}")

        for entry in pool_entries:
            pool_index = entry.get('pool_index', '?')
            name = entry.get('name', 'Unnamed')
            relative_path = entry.get('relative_path')
            media_id = entry.get('media_id')
            channels = entry.get('channels', 0)
            sample_rate = entry.get('sample_rate', 0)
            has_embedded = entry.get('embedded_data') is not None

            # Storage precedence matches the loader (§8.5/§9.3):
            # packed (media_id) > external (relative_path) > embedded > unresolved.
            row = self.media_by_id.get(media_id) if media_id else None
            if media_id:
                size = f", {_human_size(row['total_len'])}" if row else ""
                codec = f", {row['codec']}" if row else ""
                storage_type = f"Packed in DB{codec}{size}"
            elif relative_path and not self.is_sqlite and relative_path.startswith("media/audio/"):
                # Legacy ZIP: media/audio/* lives inside the archive, not external.
                storage_type = "Embedded (in ZIP)"
            elif relative_path:
                storage_type = "External reference"
            elif has_embedded:
                storage_type = "Embedded (inline base64)"
            else:
                storage_type = "Unresolved (missing)"

            print(f"\n  [{pool_index}] {name}")
            if media_id:
                print(f"    Media ID:    {media_id}")
            print(f"    Path:        {relative_path if relative_path else 'N/A'}")
            print(f"    Storage:     {storage_type}")
            print(f"    Channels:    {channels}")
            print(f"    Sample Rate: {sample_rate} Hz")

    def show_media(self):
        """Display the SQLite media store (the `media` table)."""
        if not self.is_sqlite:
            print("\n(media table view applies to SQLite .beam files; "
                  "use --zip for legacy archives)", file=sys.stderr)
            return

        print("\n" + "=" * 60)
        print("MEDIA STORE")
        print("=" * 60)

        print(f"Total media rows: {len(self.media)}")

        # Summary by kind.
        by_kind: Dict[str, List[Dict[str, Any]]] = {}
        for m in self.media:
            by_kind.setdefault(MEDIA_KIND_NAMES.get(m["kind"], f"Kind {m['kind']}"), []).append(m)
        if by_kind:
            print("\nBy kind:")
            for kind_name, rows in sorted(by_kind.items()):
                packed = sum(r["total_len"] for r in rows if r["storage"] == 0)
                print(f"  {kind_name:<12} {len(rows):>4}  ({_human_size(packed)} packed)")

        print(f"\n{'Kind':<12} {'Storage':<11} {'Codec':<6} {'Size':>10}  UUID / details")
        print("-" * 78)
        for m in self.media:
            kind = MEDIA_KIND_NAMES.get(m["kind"], f"Kind {m['kind']}")
            storage = MEDIA_STORAGE_NAMES.get(m["storage"], f"Stor {m['storage']}")
            size = _human_size(m["total_len"]) if m["storage"] == 0 else "-"
            detail = m["uuid"]
            extra = []
            if m["channels"] is not None:
                extra.append(f"{m['channels']}ch@{m['sample_rate']}Hz")
            if m["width"] is not None:
                extra.append(f"{m['width']}x{m['height']}")
            if m["storage"] == 1 and m["ext_path"]:
                extra.append(f"-> {m['ext_path']}")
            if extra:
                detail += "  " + " ".join(extra)
            print(f"{kind:<12} {storage:<11} {m['codec']:<6} {size:>10}  {detail}")

    def show_container(self):
        """Show whichever container structure applies to this file."""
        if self.is_sqlite:
            self.show_media()
        else:
            self.show_zip_structure()

    def show_zip_structure(self):
        """Display the ZIP file structure."""
        if self.is_sqlite:
            print("\n(this is a SQLite .beam; use --media for the media store)",
                  file=sys.stderr)
            return

        print("\n" + "=" * 60)
        print("ZIP ARCHIVE STRUCTURE")
        print("=" * 60)

        try:
            with zipfile.ZipFile(self.beam_file, 'r') as zip_ref:
                total_size = 0
                compressed_size = 0

                print(f"{'File':<40} {'Size':>12} {'Compressed':>12} {'Method':<10}")
                print("-" * 80)

                for info in zip_ref.infolist():
                    size = info.file_size
                    comp_size = info.compress_size
                    method = "DEFLATE" if info.compress_type == 8 else "STORED" if info.compress_type == 0 else f"Type {info.compress_type}"

                    total_size += size
                    compressed_size += comp_size

                    print(f"{info.filename:<40} {size:>12,} {comp_size:>12,} {method:<10}")

                print("-" * 80)
                print(f"{'TOTAL':<40} {total_size:>12,} {compressed_size:>12,}")

                if total_size > 0:
                    ratio = (1 - compressed_size / total_size) * 100
                    print(f"\nCompression Ratio: {ratio:.1f}%")

        except Exception as e:
            print(f"Error reading ZIP structure: {e}", file=sys.stderr)

    def extract_json(self, output_path: Optional[Path] = None):
        """Extract project.json to a file or stdout."""
        try:
            if self.is_sqlite:
                con = self._connect()
                try:
                    row = con.execute("SELECT data FROM project_json WHERE id = 0").fetchone()
                finally:
                    con.close()
                if not row:
                    raise ValueError("archive has no project.json row")
                json_data = row[0].encode("utf-8") if isinstance(row[0], str) else row[0]
            else:
                with zipfile.ZipFile(self.beam_file, 'r') as zip_ref:
                    json_data = zip_ref.read('project.json')

            if output_path:
                output_path.write_bytes(json_data)
                print(f"Extracted project.json to: {output_path}")
            else:
                data = json.loads(json_data)
                print(json.dumps(data, indent=2))

        except Exception as e:
            print(f"Error extracting project.json: {e}", file=sys.stderr)

    def extract_media(self, output_dir: Path):
        """Extract all media to a directory.

        SQLite: each packed media row is reassembled from its chunks and written
        as ``<uuid>.<codec>``; referenced rows are reported, not copied.
        ZIP (legacy): the ``media/`` entries are extracted preserving paths.
        """
        try:
            if self.is_sqlite:
                self._extract_media_sqlite(output_dir)
            else:
                self._extract_media_zip(output_dir)
        except Exception as e:
            print(f"Error extracting media: {e}", file=sys.stderr)

    def _extract_media_sqlite(self, output_dir: Path):
        con = self._connect()
        try:
            rows = con.execute(
                "SELECT id, kind, codec, storage, ext_path, total_len FROM media"
            ).fetchall()
            if not rows:
                print("No media rows found in archive.")
                return
            output_dir.mkdir(parents=True, exist_ok=True)
            written = 0
            for (idb, kind, codec, storage, ext_path, total_len) in rows:
                uid = str(uuidlib.UUID(bytes=bytes(idb)))
                kind_name = MEDIA_KIND_NAMES.get(kind, f"kind{kind}")
                if storage == 1:  # Referenced
                    print(f"Referenced (not copied): {uid} [{kind_name}] -> {ext_path}")
                    continue
                chunks = con.execute(
                    "SELECT bytes FROM media_chunk WHERE media_id = ? ORDER BY chunk_index",
                    (idb,),
                ).fetchall()
                data = b"".join(bytes(c[0]) for c in chunks)
                out = output_dir / f"{uid}.{codec}"
                out.write_bytes(data)
                print(f"Extracted: {out.name}  [{kind_name}] {_human_size(len(data))}")
                written += 1
            print(f"\nExtracted {written} packed media item(s) to: {output_dir}")
        finally:
            con.close()

    def _extract_media_zip(self, output_dir: Path):
        with zipfile.ZipFile(self.beam_file, 'r') as zip_ref:
            media_files = [f for f in zip_ref.namelist() if f.startswith('media/')]
            if not media_files:
                print("No media files found in archive.")
                return
            output_dir.mkdir(parents=True, exist_ok=True)
            for media_file in media_files:
                output_path = output_dir / media_file
                output_path.parent.mkdir(parents=True, exist_ok=True)
                with zip_ref.open(media_file) as source:
                    output_path.write_bytes(source.read())
                print(f"Extracted: {media_file}")
            print(f"\nExtracted {len(media_files)} media file(s) to: {output_dir}")


def main():
    parser = argparse.ArgumentParser(
        description="Inspect Lightningbeam .beam project files",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  %(prog)s project.beam                    # Show all information
  %(prog)s project.beam --info             # Show only basic info
  %(prog)s project.beam --tracks           # Show only tracks
  %(prog)s project.beam --media            # Show the SQLite media store
  %(prog)s project.beam --zip              # Show legacy ZIP structure
  %(prog)s project.beam --extract-json     # Print project.json to stdout
  %(prog)s project.beam --extract-json out.json  # Save project.json
  %(prog)s project.beam --extract-media ./media  # Extract media files

Handles both current SQLite .beam files and legacy ZIP .beam files
(detected automatically by the file's magic bytes).
        """
    )

    parser.add_argument('beam_file', type=Path, help='.beam file to inspect')
    parser.add_argument('--info', action='store_true', help='Show project information')
    parser.add_argument('--clips', action='store_true', help='Show clips')
    parser.add_argument('--layers', action='store_true', help='Show layer hierarchy')
    parser.add_argument('--tracks', action='store_true', help='Show audio tracks')
    parser.add_argument('--graphs', action='store_true', help='Show node graph details for all tracks')
    parser.add_argument('--pool', action='store_true', help='Show audio pool')
    parser.add_argument('--media', action='store_true', help='Show the SQLite media store')
    parser.add_argument('--zip', action='store_true', help='Show legacy ZIP structure')
    parser.add_argument('--extract-json', nargs='?', const=True, metavar='OUTPUT',
                       help='Extract project.json (to file or stdout)')
    parser.add_argument('--extract-media', type=Path, metavar='DIR',
                       help='Extract media files to directory')

    args = parser.parse_args()

    # Validate input file
    if not args.beam_file.exists():
        print(f"Error: File not found: {args.beam_file}", file=sys.stderr)
        sys.exit(1)

    if not args.beam_file.suffix == '.beam':
        print(f"Warning: File does not have .beam extension: {args.beam_file}", file=sys.stderr)

    # Create inspector
    inspector = BeamInspector(args.beam_file)

    # Handle extract operations (don't need to load project.json)
    if args.extract_json:
        if args.extract_json is True:
            inspector.extract_json()
        else:
            inspector.extract_json(Path(args.extract_json))
        return

    if args.extract_media:
        inspector.extract_media(args.extract_media)
        return

    # Load the beam file
    if not inspector.load():
        sys.exit(1)

    # If no specific flags, show everything
    show_all = not any([args.info, args.clips, args.layers, args.tracks,
                        args.graphs, args.pool, args.media, args.zip])

    if show_all or args.info:
        inspector.show_info()

    if show_all or args.clips:
        inspector.show_clips()

    if show_all or args.layers:
        inspector.show_layers()

    if show_all or args.tracks:
        inspector.show_tracks()

    if show_all or args.graphs:
        inspector.show_graphs()

    if show_all or args.pool:
        inspector.show_audio_pool()

    if show_all:
        # Show whichever container structure applies to this file.
        inspector.show_container()
    elif args.media:
        inspector.show_media()
    elif args.zip:
        inspector.show_zip_structure()


if __name__ == '__main__':
    main()
