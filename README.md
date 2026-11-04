<div align="center">
  <img src="docs/images/logo.png" alt="Kaye logo" width="128" height="128">
  <h1>Kaye</h1>
  <p>A Blender-style animation tool for Roblox</p>
  <p>
    <a href="https://github.com/Toorium/Kaye/releases/latest"><img src="https://img.shields.io/github/v/release/Toorium/Kaye?label=release&color=F36B70" alt="Latest release"></a>
    <a href="https://github.com/Toorium/Kaye/releases"><img src="https://img.shields.io/github/downloads/Toorium/Kaye/total?color=F36B70" alt="Downloads"></a>
    <a href="LICENSE"><img src="https://img.shields.io/github/license/Toorium/Kaye" alt="License"></a>
    <img src="https://img.shields.io/badge/platform-Windows-0078D6" alt="Windows">
  </p>
  <p>
    <a href="https://github.com/Toorium/Kaye/releases/latest"><b>Download</b></a>
    ·
    <a href="https://github.com/Toorium/Kaye/releases">All releases</a>
    ·
    <a href="https://github.com/Toorium/Kaye/issues">Report a bug</a>
  </p>
</div>

![Kaye with an R15 rig mid-run, its right shoulder selected and keys on the timeline](docs/images/hero.png)

Kaye is a free animation program made only for Roblox. Open your rig's `.rbxm`, animate it, and drag the result straight back into Studio. No add-ons to install, no converting files, no rig to rebuild.

Blender can animate anything, which is why it takes a 300 MB download, a long list of menus and a pile of plugins before a Roblox rig even moves in it. Kaye keeps the parts of Blender animators like (G and R to move and rotate, `R R` to tumble, auto-smooth Bezier curves, a Graph Editor, a dark 3D view) and drops everything that isn't Roblox animation. If you already know Blender, you'll feel at home in minutes. If you don't, there's far less to learn.

### Why not just use Blender?

| | Kaye | Blender |
| --- | --- | --- |
| Opens `.rbxm` and `.rbxmx` from Studio | Yes, drag the file in | Needs a plugin or a converted file |
| Exports a `KeyframeSequence` Studio can load | Yes, one menu item | Needs a plugin |
| Drag a rig from the app straight into Studio | Yes | No |
| Loads animations saved in a rig's `AnimSaves` | Yes | No |
| IK on R6, R15 and custom rigs with no rig setup | Yes | You build the IK rig yourself |
| Roblox's easing styles from a right click | Yes | No |
| Roblox's real materials, meshes and textures | Yes, downloaded from Roblox | Imported by hand |
| Installer size | 21 MB | Over 300 MB |
| Window open in | Under a second | Several seconds |
| Runs smoothly on a laptop with integrated graphics | Yes, 120 fps average | Struggles with heavy scenes |

## Features

**Rigs from Studio**

- Opens `.rbxm` and `.rbxmx` files: R6, R15, Rthro, custom Motor6D rigs and skinned rigs, with their hats and other accessories attached
- Brings in animations already saved in a rig's `AnimSaves`
- Shows real meshes, textures and clothing once you give it a Roblox Open Cloud key, and Roblox's own materials with no key at all

**Posing**

- On-screen handles, Blender's G and R keys, or typed numbers, with undo for all of it
- Box select, and moving or turning several bones at once
- IK with poles and bend limits on R6, R15 and custom rigs, plus Look At, Copy Bone, Limit Rotation, Inherit Rotation and Ragdoll
- Switching a limb between IK and FK, or unparenting it, without it moving
- Copy, paste and mirror poses

**Animating**

- Timeline with per-axis keys, playback, loops and Smooth Loop
- Auto-smooth Bezier curves by default, and every Roblox easing style from a right click
- A Graph Editor where you drag keys and handles on every channel
- Onion skins, motion paths, and any number of animations per rig
- Cameras you can animate, field of view included, with a corner preview

**Back to Roblox**

- Exports a `KeyframeSequence`, on its own or inside the rig's model
- Drag rigs, cameras and shapes from the Workspace straight into Studio

**Everything else**

- `.kaye` project files, and a backup of your work every 5 minutes
- Studio and Shaded lighting, themes where every color can be changed, and rebindable keys
- Luau plugins from a Plugins menu
- Tells you when a new version is out, and updates itself

<table>
  <tr>
    <td width="33%"><img src="docs/images/graph.png" alt="The Graph Editor showing a shoulder's rotation curve"></td>
    <td width="33%"><img src="docs/images/avatar-onion.png" alt="An onion skin in green over a real avatar with accessories, clothing and a body package"></td>
    <td width="33%"><img src="docs/images/materials.jpg" alt="Blocks in Roblox's materials on a Baseplate"></td>
  </tr>
  <tr>
    <td align="center">Graph Editor</td>
    <td align="center">Onion skins</td>
    <td align="center">Roblox materials</td>
  </tr>
</table>

<table>
  <tr>
    <td width="50%"><img src="docs/images/avatar-shaded.png" alt="A real avatar in Shaded mode, with soft lighting and the bones hidden"></td>
    <td width="50%"><img src="docs/images/avatar-studio.png" alt="The same avatar in Studio mode, lit like Roblox Studio, with its bones showing"></td>
  </tr>
  <tr>
    <td align="center">Shaded mode</td>
    <td align="center">Studio mode</td>
  </tr>
</table>

## Performance

Kaye is light enough for any PC. On an ASUS Vivobook 15 OLED laptop with a Core i3, Intel UHD integrated graphics and 8 GB of memory, it averaged **120 fps with no lag**.

- Draws its first frame 0.22 seconds after it starts
- Uses about 150 MB of memory with a rig open
- Uses no CPU at all while you're not touching it, so it won't drain a laptop's battery in the background

### Measured

On the development PC: an Intel Core i5-12400F, Intel Arc B570 and 16 GB of memory, at 1600 by 900 with vsync off so nothing caps the number.

| What's on screen | FPS | Frame time |
| --- | --- | --- |
| R15 rig sitting still | 1,218 | 0.78 ms |
| R15 rig, Shaded lighting with shadows | 1,044 | 0.91 ms |
| R15 rig playing, IK on all four limbs | 964 | 1.00 ms |
| R15 rig playing, 120 keys per bone | 492 | 1.87 ms |
| Graph Editor open, playing 120 keys | 278 | 3.40 ms |
| 20 rigs playing, IK and poles on every limb | 333 | 2.79 ms |
| 50 rigs playing, IK and poles on every limb | 165 | 5.90 ms |
| 10,000 parts, Workspace open | 408 | 2.26 ms |

A 60 Hz screen needs a frame every 16.7 ms, so one rig uses about a sixteenth of that. Idle, Kaye drew 2 frames in 10 seconds. Over a minute of playback, memory moved by under 1 MB.

### Other hardware

These are predictions, worked out from the measurements above. The graphics card only spends up to about 0.5 ms on each frame here. The rest is one CPU core running the interface. So each frame is split into a CPU part, scaled by the CPU's single-core speed (its Cinebench R23 single-core score), and a GPU part, scaled by the card's speed (3DMark Time Spy graphics score):

```text
frame time = CPU part × (1700 ÷ your CPU's score) + GPU part × (12,500 ÷ your GPU's score)
fps        = 1000 ÷ frame time
```

1700 and 12,500 are the i5-12400F and Arc B570. The GPU part is set to 0.5 ms, the top of what was measured, which brings the Vivobook's Core i3 with Intel UHD to about 130 fps against the 120 it really measured. That run was before later speedups, so Kaye has likely got faster on it since.

| Hardware | One rig, IK, playing | Graph Editor open | 10 rigs with IK |
| --- | --- | --- | --- |
| Core i7-14700K + RTX 4090 | ~1,730 | ~490 | ~930 |
| Ryzen 7 7800X3D + RTX 4070 | ~1,210 | ~390 | ~700 |
| **Core i5-12400F + Arc B570 (measured)** | **964** | **~360** | **~600** |
| Ryzen 5 5600 + RTX 3060 | ~780 | ~300 | ~480 |
| Core i3-10100 + GTX 1650 | ~400 | ~200 | ~270 |
| Ryzen 7 7840U, Radeon 780M (laptop) | ~380 | ~220 | ~270 |
| Core i5-1135G7, Iris Xe (laptop) | ~230 | ~150 | ~170 |
| Ryzen 5 3500U, Vega 8 (laptop) | ~140 | ~90 | ~100 |
| **Core i3, Intel UHD (Vivobook 15 OLED, measured 120)** | **~130** | **~100** | **~100** |
| Core i5-8250U, UHD 620 (laptop) | ~70 | ~50 | ~50 |
| Celeron N4020, UHD 600 | ~30 | ~20 | ~20 |

With vsync on, which is the default, Kaye stops at your screen's refresh rate, so anything above 60 on a 60 Hz screen just means room to spare. Only the oldest laptops at the bottom dip below a 60 Hz screen, and they stay usable for posing.

## Download

Get `Kaye-<version>.msi` from the [latest release](https://github.com/Toorium/Kaye/releases/latest) and run it. Nothing needs building. It installs for your Windows account only, in `%LOCALAPPDATA%\Programs\Kaye`, so it doesn't ask for administrator rights.

Kaye isn't code signed yet, so Windows may say it protected your PC. Click **More info**, then **Run anyway**.

If you'd rather not install anything, `kaye.exe` on the same page is the whole program and runs from any folder.

From 0.1.1 on, Kaye checks for a new version when it starts. Click **Update** and it closes, downloads only the files that changed, and opens again. Your projects, settings and backups aren't touched.

### Requirements

| | Recommended |
| --- | --- |
| System | Windows 10 or 11, 64-bit |
| Processor | Intel Core i3 or better |
| Graphics | Integrated graphics or better, with Direct3D 12, Vulkan or OpenGL |
| Memory | 4 GB |
| Disk | About 60 MB, plus a cache for downloaded meshes and textures |

## Getting started

1. Open Kaye. It asks for a Roblox Open Cloud API key and walks you through making one, which takes about a minute. Without a key, rigs still open, but their parts show as plain grey boxes. The key stays on your computer, in Windows Credential Manager.
2. In Studio, right-click your rig and pick **Save to File** to get a `.rbxm`.
3. Drag that file onto Kaye's window, or use the Import button.
4. Animate. Auto Key is on, so posing a bone keys it on the current frame.
5. Use **File > Export > .rbxm > Rig** for the rig with its animations in `AnimSaves`, or **Keyframe** for the animation alone, and drop the file into Studio.

## Building from source

You need the latest stable [Rust](https://rustup.rs), and the Visual Studio C++ build tools, since the Luau plugin runtime is C++.

```bash
git clone https://github.com/Toorium/Kaye.git
cd Kaye
cargo run --release
```

Run the tests:

```bash
cargo test
```

Build the installer, which needs [WiX 3](https://github.com/wixtoolset/wix3/releases) and lands in `dist\`:

```bash
powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
```

Publish a release, after raising the version in `Cargo.toml` and committing:

```bash
powershell -ExecutionPolicy Bypass -File scripts\publish-update.ps1 -Notes "What's new"
```

## Project status

Kaye is early. Releases are test builds, and things will break. If something does, an [issue](https://github.com/Toorium/Kaye/issues) with the `.rbxm` you used helps the most.

- Windows only for now. Mac and Linux builds have never been tried.
- Exported animations haven't all been checked in Studio yet, and a camera's animated field of view doesn't export.
- Decals only show on plain blocks, and ragdolls don't collide with themselves.

## Credits

Kaye is built on [egui](https://github.com/emilk/egui) and [wgpu](https://github.com/gfx-rs/wgpu), reads and writes Roblox files with [rbx-dom](https://github.com/rojo-rbx/rbx-dom), and runs plugins with [mlua](https://github.com/mlua-rs/mlua) and [Luau](https://github.com/luau-lang/luau). Icons are from [Phosphor](https://phosphoricons.com) under the MIT license.

## License

Kaye is released under the BSD 3-Clause License. See [LICENSE](LICENSE).

Kaye is a fan-made tool and isn't affiliated with or endorsed by Roblox Corporation. Roblox and Roblox Studio are trademarks of Roblox Corporation.
