# 🔱 Triton

> **Status:** _alpha_ • **OS:** Windows **(tested)** · Linux/macOS **(experimental)**, PRs welcome

Tired of chanting mysterious CMake incantations, tiptoeing around vcpkg quirks, and untangling a plate of build-file spaghetti?
**Triton** is a tiny, no-nonsense C++ project manager that snaps CMake and vcpkg together like LEGO®, auto-wires your dependencies, and keeps your codebase ship-shape.. so you can spend your energy shipping features instead of waging war on your build system.

> At least, that’s the dream **Triton** is trying to live up to. It’s still alpha — so it might occasionally trip over its own trident.

If **Triton** itself feels tricky, hey — nothing’s stopping you from making your own wrapper around Triton… which would make it the fourth wrapper around C++ package managers. And who knows.. maybe someone will wrap your wrapper. Together we can summon Wrapzilla.

**Or…** you can help Triton ascend and become the most powerful of them all. PRs, issues, and wild suggestions are always welcome aboard.

## ✨ Highlights

- ✅ Define all dependencies once in `triton.json`
- 🔗 Link components via `"link": ["depName", …]`
- ⚙️ Generates the right `find_package` / `add_subdirectory` / `target_link_libraries` into **managed regions**
- 📦 vcpkg runs in **manifest mode**
- 🌱 Git deps vendored into `third_party/<name>` via `add_subdirectory(...)`
- 🪟 Windows + Ninja: uses `VsDevCmd.bat` to prime the MSVC environment

## ⚙️ Requirements

- **Windows 10/11** (tested). Linux & macOS are experimental.
- **Rust** (stable)
- **CMake** (≥ 3.25 recommended)
- **Ninja** (recommended) or MSBuild
- **Git**
- **vcpkg** (manifest mode; Triton manages `vcpkg.json`)
- **Visual Studio 2022 Build Tools** (cl.exe, link.exe) on Windows

## 🚀 Install

[🪟 Download Triton (Windows)](https://github.com/Vulcaine/triton/releases/latest/download/triton.exe)

**OR**

With Rust installed:

```bash
cargo install --path .   # run from the Triton source repo
```

## Make sure Cargo’s bin dir is on your `PATH`:

### Git Bash

```bash
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

### PowerShell

```bash
$env:Path += ";$env:USERPROFILE\.cargo\bin"
[Environment]::SetEnvironmentVariable("Path", $env:Path, "User")
```

# Quick Start

```bash
triton init --name demo
cd demo
triton add sdl2 glm               # add deps (no linking)
triton add sdl2:demo              # add+link to component 'demo'
triton build . --config debug     # default is debug
triton run .
```

## Initialize an existing repo

If you want to use triton in an existing project, that requires a bit of work.
Triton enforces a project architecture as a convention, so you must move everything under `components`.
Your earlier CMakeLists.txt will no longer be needed, triton will automatically generate them.

```bash
cd existing-repo
triton init .                     # minimal: writes triton.json and components/CMakeLists.txt
# Put your components under ./components/<name>
# Add them to triton.json -> components -> "<name>": { "kind": "...", "link": [...] }
triton generate                   # write/refresh managed CMake blocks
```

## Triton Json

```json
{
  "app_name": "demo",
  "triplet": "x64-windows",
  "generator": "Ninja",
  "cxx_std": "20",
  "deps": [
    "sdl2",
    "glm",
    { "repo": "google/filament", "name": "filament", "branch": null, "cmake": [] }
  ],
  "components": {
    "demo": {
      "kind": "exe",
      "link": [
        "sdl2",
        "glm",
        { "name": "filament", "targets": ["filament"] }
      ]
    },
    "core": {
      "kind": "lib",
      "link": ["glm"]
    }
  }
}
```

If you need multiple filament targets, use:
```json
{ "name": "filament", "targets": ["filament", "utils", "math"] }
```

### Top-level fields

| Field        | Type   | Example         | Notes                                         |
|--------------|--------|-----------------|-----------------------------------------------|
| `app_name`   | string | `"demo"`        | Default executable / main component name      |
| `triplet`    | string | `"x64-windows"` | vcpkg triplet                                 |
| `generator`  | string | `"Ninja"`       | CMake generator                               |
| `cxx_std`    | string | `"20"`          | C++ standard                                  |
| `deps`       | array  | `["sdl2", …]`   | vcpkg or Git deps (see below)                 |
| `components` | object | `{ ... }`       | Map of component configs                       |


### deps entries

| Field   | Required | Example               | Meaning                                                                 |
|---------|----------|-----------------------|-------------------------------------------------------------------------|
| `repo`  | ✓ (git)  | `"google/filament"`   | GitHub org/repo                                                         |
| `name`  | ✓ (git)  | `"filament"`          | Local folder/dep name (used for linking & `third_party/<name>`)        |
| `branch`| –        | `"v3.0.0"`            | Optional branch/tag                                                     |                             |
| `cmake` | –        | `["-DFILAMENT=ON"]`   | Optional list of cache entries injected before `add_subdirectory`       |

**`cmake` entry format (structured):**

```json
"cmake": [
  "FILAMENT_SOME_OPTION=ON"
  "CMAKE_POLICY_DEFAULT_CMP0091=NEW"
]
```

### components entries

| Field  | Type     | Allowed  | Example      | Meaning                               |
|--------|----------|----------|--------------|---------------------------------------|
| `kind` | string   | `exe/lib`| `"exe"`      | Component type                        |
| `link` | string[] | names    | `["sdl2"]`   | Names of deps or other components     |


### Commands Overview

| Command                    | Purpose                                           | Common Options / Notes                                                                                   |
|---------------------------|---------------------------------------------------|-----------------------------------------------------------------------------------------------------------|
| `triton init --name <dir>`       | Create new project scaffold in `<dir>`            | `triton init .` creates minimal files in current folder                                                   |
| `triton add ...`          | Add one or more deps; optionally link to a component | Supports `pkg`, `org/repo@branch`, `pkg->Comp`; transactional with vcpkg                                  |
| `triton link A->B`        | Component-to-component linking (creates components if missing) | New components default to `kind: "lib"`                                                        |
| `triton remove`           | Remove/unlink deps                                | `--component <name>` to only unlink from that component                                                   |
| `triton generate`         | Rewrite managed CMake regions from `triton.json`  | Safe: only edits the managed blocks                                                                        |
| `triton build .`          | Configure + build via CMake                       | `--config debug|release`                                                                                  |
| `triton run .`            | Run a built component                             | `--component <name>`, `--config ...`, `--` passes args                                                    |


# `add` examples

```bash
triton add lua sol2
triton add lua:Game sol2:Game
triton add lua sol2 Game 
triton add org/repo@tag:Renderer 
```

**Behavior**
- vcpkg deps: transactional — if `vcpkg install` fails, changes to vcpkg.json are reverted and the dep is not recorded in `triton.json`.
- Git deps: recorded only if the clone (and optional checkout) succeeds.
- Linking to a missing component scaffolds it at `components/<name>/{src,include}` with `CMakeLists.txt`, and adds it to `triton.json` with `kind: "lib"` by default.

# `remove ` examples

```bash
triton remove <pkg> # removes the package entirely from the project
triton remove <pkg> --component <name>  # removes the package linking from <name>
```

# Build/Run

```bash
triton build . --config debug # equivalent of triton build .
triton build . --config release

triton run . -- --arg1 --arg2
```


## Project Layout

| Path / File                                   | Purpose                                                                                   |
|-----------------------------------------------|-------------------------------------------------------------------------------------------|
| `your-project/`                               | Project root                                                                              |
| `triton.json`                                 | Single source of truth for deps, components, and build settings                           |
| `vcpkg.json`                                  | vcpkg manifest (managed transactionally by Triton)                                        |
| `components/`                                 | Where your components live                                                                |
| `components/CMakeLists.txt`                   | **Generated**: adds each component as a subdirectory                                      |
| `components/App/`                              | Example component directory                                                               |
| `components/App/CMakeLists.txt`               | Per-component build rules (**managed regions** inside)                                    |
| `components/App/src/ …`                       | Component sources                                                                         |
| `components/App/include/ …`                   | Component headers                                                                         |
| `components/Core/`                             | Another component                                                                         |
| `components/Core/CMakeLists.txt`              | Per-component CMake rules                                                                 |
| `third_party/`                                | Git deps cloned here, e.g. `org/repo`                                                     |
| `build/`                                      | CMake build tree(s)                                                                       |

**Managed regions** in component `CMakeLists.txt` are owned by Triton and look like:

```cmake
# ## triton:deps begin
# (generated by Triton)
# ## triton:deps end
```
Anything outside that block is yours and will not be touched.

### Scripts

Define custom commands in `triton.json`:

```json
"scripts": {
  "dev": "triton build . --config debug && triton run . --config debug",
  "fmt": "clang-format -i components/**/src/**/*.{h,hpp,c,cpp}"
}
```

**Run With**

```bash
triton fmt
triton dev
```

Script names cannot shadow built-ins (`build`, `run`, `add`, …); Triton will error if they do.


# Notes
- Windows + Ninja: Triton uses `VsDevCmd.bat` to ensure MSVC is active.
- vcpkg runs in manifest mode using your `vcpkg.json`.
- Git deps are vendored into `third_party/<name>` and added via `add_subdirectory(...)`.
- Re-run `triton generate` anytime you update `triton.json` to refresh managed regions.