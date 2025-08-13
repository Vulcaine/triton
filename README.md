# 🔱 Triton

Tiny, easy-to-use C++ project manager that wires up **CMake + vcpkg** while keeping a clean, modular layout.

## ✨ Highlights

- ✅ Define all dependencies once in `triton.json`
- 🔗 Link components via `"link": ["depName", …]`
- ⚙️ Generates the right `find_package` / `add_subdirectory` / `target_link_libraries` into **managed regions**
- 📦 vcpkg runs in **manifest mode**
- 🌱 Git deps vendored into `third_party/<name>` via `add_subdirectory(...)`
- 🪟 Windows + Ninja: uses `VsDevCmd.bat` to prime the MSVC environment

## 🚀 Install

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
triton init demo
cd demo
triton add sdl2 glm               # add deps (no linking)
triton add sdl2:demo             # add+link to component 'demo'
triton build . --config debug
triton run . --component demo
```

## Initialize an existing repo

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
| `triton init <dir>`       | Create new project scaffold in `<dir>`            | `triton init .` creates minimal files in current folder                                                   |
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
triton remove <pkg>
triton remove <pkg> --component <name> 
```

# Build/Run

```bash
triton build . --config debug
triton build . --config release

triton run . --component demo --config debug -- --arg1 --arg2
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


# Notes
- Windows + Ninja: Triton uses `VsDevCmd.bat` to ensure MSVC is active.
- vcpkg runs in manifest mode using your `vcpkg.json`.
- Git deps are vendored into `third_party/<name>` and added via `add_subdirectory(...)`.
- Re-run `triton generate` anytime you update `triton.json` to refresh managed regions.