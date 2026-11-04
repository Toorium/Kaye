# Kaye

A Blender-style animation tool built only for Roblox rigs.

This version imports `.rbxm` and `.rbxmx` models along with any animations saved in them, and animates them with keyframes, an editable graph, IK and other modifiers, and cameras. It draws real meshes and clothing with an Open Cloud API key, saves projects, runs Luau plugins, and exports animations as a `KeyframeSequence`, alone or inside the rig's model.

## Installing

Download `Kaye-<version>.msi` from [Releases](https://github.com/Toorium/Kaye/releases) and run it. It installs for the current user, in `%LOCALAPPDATA%\Programs\Kaye`, with no administrator needed.

From 0.1.1 on, Kaye checks [`latest.json`](latest.json) when it starts. If a newer version is out, it shows **New update** with **Update** and **Later**. Update closes Kaye, downloads only the files that changed, checks each one's SHA-256, swaps them in and opens Kaye again. Settings, the API key and backups in `%APPDATA%\Kaye` aren't touched.

## Building

Run it:

```bash
cargo run --release
```

Run the tests:

```bash
cargo test
```

Build the installer, which lands in `dist\`:

```bash
powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
```

Publish a new version, after raising the version in `Cargo.toml` and committing:

```bash
powershell -ExecutionPolicy Bypass -File scripts\publish-update.ps1 -Notes "What's new"
```

Windows needs the MSVC build tools, and the installer needs WiX 3.
