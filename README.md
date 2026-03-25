# Triton

> **Status:** beta (0.9.0) | **OS:** Windows (tested), Linux/macOS (experimental)

A C++ project manager that wires CMake and vcpkg together. Define dependencies in `triton.json`, and Triton handles `find_package`, `target_link_libraries`, vcpkg manifests, and git vendoring — so you don't have to.

## Requirements

- **Rust** (stable) — to build Triton
- **CMake** (>= 3.25 recommended)
- **Ninja** (recommended) or MSBuild
- **Git**
- **vcpkg** (Triton manages `vcpkg.json` in manifest mode)
- **Visual Studio 2022 Build Tools** (Windows only)

> Triton can auto-install **Ninja**, **vcpkg**, and **CMake** if they're missing.

## Install

Download the latest binary:

[Download triton.exe (Windows)](https://github.com/Vulcaine/triton/releases/latest/download/triton.exe)

Or build from source:

```bash
cargo install --path .
```

Make sure `~/.cargo/bin` is on your PATH:

```bash
# Git Bash
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc && source ~/.bashrc

# PowerShell
$env:Path += ";$env:USERPROFILE\.cargo\bin"
[Environment]::SetEnvironmentVariable("Path", $env:Path, "User")
```

## Quick Start

```bash
triton init --name myapp          # scaffold a new project
cd myapp
triton add sdl2 glm               # add vcpkg deps
triton add sdl2:myapp glm:myapp   # add + link to component 'myapp'
triton build .                    # configure + build (debug by default)
triton run .                      # run the default executable
```

## Commands

| Command | What it does |
|---------|-------------|
| `triton init --name <dir>` | Create a new project in `<dir>` |
| `triton init .` | Minimal init in current directory |
| `triton add <deps...>` | Add dependencies, optionally link to components |
| `triton remove <dep>` | Remove a dependency entirely |
| `triton link <A>:<B>` | Link dep or component A to component B |
| `triton generate` | Regenerate CMake files from `triton.json` |
| `triton build <path>` | Configure + build |
| `triton run <path>` | Run a built component |
| `triton test <path>` | Run tests via CTest |
| `triton find-target <dep>` | Search for a dep's CMake package name |
| `triton cmake install` | Install or upgrade CMake |
| `triton <script>` | Run a custom script defined in `triton.json` |

### `init`

```bash
triton init --name demo                    # new project in demo/
triton init --name demo --generator Ninja  # specify CMake generator
triton init --name demo --cxx_std 23       # specify C++ standard
triton init .                              # init in current dir (no scaffold)
```

### `add`

```bash
triton add lua sol2                  # add deps (no linking)
triton add lua:Game sol2:Game        # add + link to component 'Game'
triton add org/repo                  # add git dependency
triton add org/repo@v1.0             # git dep with branch/tag
triton add org/repo@v1.0:Renderer    # git dep + link to component
```

- **vcpkg deps** are transactional — if `vcpkg install` fails, `vcpkg.json` is reverted and the dep is not recorded.
- **Git deps** are recorded only if the clone (and optional checkout) succeeds.
- Linking to a non-existent component auto-scaffolds it as a `lib`.
- **Auto-detection**: After install, Triton scans `vcpkg/installed/<triplet>/share/` to discover the correct CMake package name. If the package name differs from the dep name (e.g., `openal-soft` installs as `OpenAL`), Triton automatically sets the `package` field.

### `remove`

```bash
triton remove lua                       # remove entirely from project
triton remove lua --component Game      # unlink from specific component only
```

### `link`

```bash
triton link sdl2:Game          # link dep to component
triton link Core:Game          # link component to component
```

Creates missing components as `lib` by default.

### `build`

```bash
triton build .                    # debug (default)
triton build . --config release   # release
triton build . --clean            # clean build dir (prompts first)
triton build . --cleanf           # force clean (no prompt)
```

If a `pre_build` script is defined in `triton.json`, it runs automatically before each build.

### `run`

```bash
triton run .                          # run default component
triton run . --component myapp        # run specific component
triton run . --config release         # run release build
triton run . -- --arg1 --arg2         # pass args to the executable
```

### `test`

```bash
triton test .                     # run tests (debug)
triton test . --config release    # run tests (release)
```

Environment variables for test filtering:

| Variable | Purpose |
|----------|---------|
| `TRITON_CTEST_LABEL` | Filter by label (default: `triton`) |
| `TRITON_CTEST_FILTER` | Regex filter for test names |
| `TRITON_CTEST_EXCLUDE` | Regex exclude pattern |
| `TRITON_CTEST_JOBS` | Parallel job count |

### `find-target`

Debug command to discover what CMake package name vcpkg uses for a dependency.

```bash
triton find-target openal-soft    # → Found: OpenAL
triton find-target sdl2           # → Found multiple: SDL2, SDL2_mixer, SDL2_image, ...
triton find-target directxtex     # → Found: DirectXTex
```

Scans `vcpkg/installed/<triplet>/share/` for Config.cmake files and matches them against the dep name using case-insensitive and hyphen/underscore normalization. When a match is found, it shows the suggested `triton.json` entry.

---

## Validation

Triton validates your `triton.json` before generating CMake or building. You'll get clear errors for:

| Error | Example |
|-------|---------|
| Invalid component kind | `"kind": "shared_lib"` — must be `"exe"` or `"lib"` |
| Self-linking | Component `Core` links to itself |
| Circular dependencies | `A -> B -> A` cycle detected |
| Unknown link targets | Component links to `ghost_dep` which isn't in deps or components |
| Empty app_name | `"app_name": ""` |
| Missing vcpkg features | Requested feature `dx12` not installed by vcpkg |

---

## `triton.json` Reference

This is the single source of truth for your project.

### Minimal example

```json
{
  "app_name": "myapp",
  "generator": "Ninja",
  "cxx_std": "20",
  "deps": ["sdl2", "glm"],
  "components": {
    "myapp": {
      "kind": "exe",
      "link": ["sdl2", "glm"]
    }
  }
}
```

### Full example

```json
{
  "app_name": "myapp",
  "generator": "Ninja",
  "cxx_std": "20",
  "deps": [
    "sdl2",
    "glm",
    {
      "repo": "google/filament",
      "name": "filament",
      "branch": "v1.63",
      "cmake": ["FILAMENT_ENABLE_JAVA=OFF"]
    },
    {
      "name": "protobuf",
      "os": ["windows", "linux"],
      "features": ["lite"],
      "package": "Protobuf"
    }
  ],
  "components": {
    "myapp": {
      "kind": "exe",
      "link": [
        "sdl2",
        "glm",
        "core",
        { "name": "filament", "targets": ["filament", "utils"] },
        { "name": "rmlui", "package": "RmlUi", "targets": ["RmlUi::RmlUi"] }
      ],
      "defines": ["APP_VERSION=1"],
      "exports": ["glm"],
      "resources": ["resources"],
      "assets": ["data", "config.json"],
      "link_options": ["-Wl,--export-dynamic"],
      "vendor_libs": ["vendor/libfoo.a"]
    },
    "core": {
      "kind": "lib",
      "link": ["glm"],
      "defines": ["CORE_LIB"],
      "exports": ["glm"]
    }
  },
  "scripts": {
    "dev": "triton build . --config debug && triton run . --config debug",
    "fmt": "clang-format -i components/**/src/**/*.{h,hpp,c,cpp}",
    "pre_build": "bash scripts/generate-version.sh"
  }
}
```

### Top-level fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `app_name` | string | yes | Default executable / main component name |
| `generator` | string | yes | CMake generator (`Ninja`, `Unix Makefiles`, etc.) |
| `cxx_std` | string | yes | C++ standard (`17`, `20`, `23`) |
| `deps` | array | no | vcpkg and git dependencies |
| `components` | object | no | Component definitions |
| `scripts` | object | no | Custom scripts |

### Dependency formats

**Simple vcpkg dep** — just a string:
```json
"deps": ["sdl2", "glm"]
```

**vcpkg dep with options:**
```json
{
  "name": "protobuf",
  "os": ["windows", "linux"],
  "triplet": ["x64-windows"],
  "features": ["lite"],
  "package": "Protobuf"
}
```

| Field | Required | Description |
|-------|----------|-------------|
| `name` | yes | Package name |
| `os` | no | Restrict to OS list (`windows`, `linux`, `macos`) |
| `triplet` | no | Restrict to vcpkg triplets |
| `features` | no | vcpkg features to enable |
| `package` | no | Override the `find_package()` name (auto-detected when possible) |

**Git dep:**
```json
{
  "repo": "google/filament",
  "name": "filament",
  "branch": "v1.63",
  "cmake": [
    "FILAMENT_ENABLE_JAVA=OFF",
    { "var": "CMAKE_BUILD_TYPE", "val": "Release", "typ": "STRING" }
  ]
}
```

| Field | Required | Description |
|-------|----------|-------------|
| `repo` | yes | GitHub `org/repo` |
| `name` | yes | Local name (used for `third_party/<name>`) |
| `branch` | no | Branch or tag to checkout |
| `cmake` | no | CMake cache variables set before `add_subdirectory` |

### Component fields

| Field | Type | Description |
|-------|------|-------------|
| `kind` | `"exe"` or `"lib"` | Component type (required) |
| `link` | array | Deps and components to link against |
| `defines` | string[] | Preprocessor defines (`"KEY=VALUE"`) |
| `exports` | string[] | Re-export these deps PUBLIC to dependents |
| `resources` | string[] | Directories copied next to executable on build |
| `assets` | string[] | Files/dirs staged incrementally (only copies changes) |
| `link_options` | string[] or object | Linker flags (see below) |
| `vendor_libs` | string[] or object | Pre-built library files (see below) |

**Link entries** support multiple formats:

```json
"link": [
  "sdl2",
  { "name": "filament", "targets": ["filament", "utils"] },
  { "name": "rmlui", "package": "RmlUi", "targets": ["RmlUi::RmlUi"] }
]
```

**Platform-specific `link_options` and `vendor_libs`:**

```json
"link_options": {
  "linux": ["-Wl,--export-dynamic"],
  "windows": [],
  "macos": []
}

"vendor_libs": {
  "linux": ["vendor/libnethost.a"],
  "windows": ["vendor/nethost.lib"]
}
```

Or use a flat array to apply to all platforms.

### Scripts

```json
"scripts": {
  "dev": "triton build . && triton run .",
  "fmt": "clang-format -i components/**/src/**/*.{h,hpp,c,cpp}",
  "pre_build": "bash scripts/setup.sh"
}
```

- Run with `triton <script-name>`
- Script names cannot shadow built-in commands
- `pre_build` is special: it runs automatically before `triton build`

---

## Project Layout

```
myapp/
├── triton.json                  # your project config
├── vcpkg.json                   # managed by Triton
├── components/
│   ├── CMakeLists.txt           # generated: adds subdirectories
│   ├── CMakePresets.json        # generated: build presets
│   ├── myapp/
│   │   ├── CMakeLists.txt       # managed regions inside
│   │   ├── src/
│   │   └── include/
│   └── core/
│       ├── CMakeLists.txt
│       ├── src/
│       └── include/
├── third_party/                 # git deps cloned here
└── build/
    ├── debug/
    └── release/
```

### Managed regions

Triton owns specific blocks inside component `CMakeLists.txt` files:

```cmake
# ## triton:deps begin
# (generated by Triton — do not edit)
# ## triton:deps end
```

Everything outside these blocks is yours and will never be touched. Run `triton generate` to refresh them after manually editing `triton.json`.

> **Warning:** Avoid editing `CMakeLists.txt` files directly. Triton is designed to handle all CMake configuration through `triton.json` — manual CMake edits should be a last resort. If you find yourself needing to touch CMake directly, consider whether the change belongs in `triton.json` instead (e.g. `defines`, `link_options`, `exports`, `vendor_libs`, `resources`, `assets`).

## Initializing an Existing Project

```bash
cd existing-repo
triton init .
# Move your code under components/<name>/src/ and components/<name>/include/
# Add components to triton.json
triton generate
```

Your old `CMakeLists.txt` at the project root is replaced by Triton's generated files.

## Contributing

PRs, issues, and suggestions welcome. Linux/macOS support is experimental — contributions especially appreciated there.
