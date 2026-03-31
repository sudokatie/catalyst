# Catalyst

A build system that actually cares about reproducibility.

## Why This Exists

Because "works on my machine" stopped being funny in 2015.

Catalyst is a build system inspired by Bazel's best ideas, minus the JVM startup time that lets you make coffee while waiting. It provides:

- **Content-addressed caching** - Same inputs, same outputs. Always. SHA-256 doesn't lie.
- **Hermetic builds** - Your build won't secretly depend on that random thing you installed three months ago.
- **Parallel execution** - DAG-based scheduling means independent actions run concurrently. Your cores will finally earn their keep.
- **Starlark-like BUILD files** - Declarative, readable, version-controllable.

## Features

- Parse and resolve BUILD files with dependency tracking
- Content-addressed storage for build artifacts
- Action caching with mtime-based invalidation
- Parallel worker pool with configurable concurrency
- DAG scheduler that respects dependency order
- Built-in rules for Rust, C/C++, and custom commands
- Query API for dependency analysis
- DOT graph export for visualization

## Quick Start

```bash
# Build
cargo install --path .

# Show configuration
catalyst info

# Build a target
catalyst build //app:main

# Query dependencies
catalyst query deps //app:main

# Clean build outputs
catalyst clean
```

## BUILD File Syntax

```python
# Variables work
COMMON_DEPS = [":util", ":logging"]

# Rust binary
rust_binary(
    name = "myapp",
    srcs = ["src/main.rs"],
    deps = [":mylib"] + COMMON_DEPS,
)

# Rust library
rust_library(
    name = "mylib",
    srcs = ["src/lib.rs"],
)

# Custom command
genrule(
    name = "generate_config",
    srcs = ["config.template"],
    outs = ["config.json"],
    cmd = "process $< > $@",
)

# Group files
filegroup(
    name = "assets",
    srcs = glob(["assets/**/*"]),
)
```

## Configuration

Create `.catalystrc` in your workspace or home directory:

```toml
[build]
jobs = 8          # Parallel jobs (0 = auto-detect)
sandbox = true    # Enable sandboxing

[cache]
local = "~/.catalyst/cache"
# remote = "grpc://cache.example.com:8080"
```

Environment overrides:
- `CATALYST_JOBS` - Number of parallel jobs
- `CATALYST_SANDBOX` - Enable/disable sandboxing
- `CATALYST_CACHE_DIR` - Local cache directory

## CLI Commands

```
catalyst build <targets>       Build specified targets
catalyst test <targets>        Build and run tests
catalyst run <target> [args]   Build and run binary
catalyst query <mode> <target> Query dependency graph (deps, rdeps)
catalyst clean                 Remove build outputs
catalyst gc                    Garbage collect cache
catalyst info                  Show build configuration
```

## Architecture

```
CLI Layer (build, test, query, clean)
         |
   Build Engine
  /      |      \
Parser  Resolver  Scheduler
   |      |           |
  AST   Graph    WorkerPool
         |           |
     Targets     Executor
         |           |
        Rules    Actions
                     |
                  Cache
               /    |    \
           CAS   ActionCache  Metadata
```

## Philosophy

1. **Reproducibility over convenience** - If it's not reproducible, it's not a build system, it's a prayer.
2. **Explicit over implicit** - Dependencies are declared, not discovered. Magic is for wizards.
3. **Cache everything** - The fastest build is the one that doesn't run.
4. **Parallelize everything else** - When you must build, use all the cores.

## Status

This is a working implementation of core build system concepts. It can:
- Parse BUILD files with full expression support
- Resolve dependencies and detect cycles
- Generate correct build commands
- Cache results by content hash
- Execute actions in parallel

What's not implemented (yet):
- Remote caching
- Remote execution
- Full sandboxing (namespace isolation on Linux)
- Incremental compilation

## License

MIT

---

*Built by Katie, who has strong opinions about build systems.*
