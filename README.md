# Clay

Fast Node.js package manager written in Rust with revolutionary content-addressable dependency resolution.

## Why Clay?

Clay is moldable and adaptive - just like this package manager that shapes itself to your project's needs. Clay can be formed into any container, much like how this tool manages and contains your dependencies efficiently.

## Revolutionary Architecture: Lockfile-Free Reproducibility

Clay introduces **content-addressable dependency fingerprinting** - the first package manager to achieve perfect reproducibility without lockfiles:

- **Zero lockfile clutter** - no `clay-lock.*` files in your projects
- **Dependency fingerprinting** - deterministic hashes calculated from `package.json`
- **Global content store** - resolved dependency trees cached across all projects
- **Perfect reproducibility** - same dependencies = same fingerprint = identical resolution
- **Team-friendly** - no version control conflicts from lockfile changes

### How It Works

```bash
# First install: resolve dependencies → store in global content store
clay install react

# Later installs: same fingerprint → cached resolution → instant install
clay install  # Uses cached dependency tree - blazing fast!
```

**Your project stays clean** - only `package.json` and `node_modules`, nothing else.

## Features

- Content-addressable storage with automatic deduplication
- Lightning-fast installations with parallel dependency resolution  
- Intelligent dependency fingerprinting (no lockfile clutter)
- Built-in bundler with TypeScript support
- Development server with hot module replacement
- Monorepo/workspace support
- 50-80% storage savings through deduplication

## Quick Start

```bash
# Install Clay
curl -fsSL https://raw.githubusercontent.com/lassejlv/clay/main/scripts/install.sh | bash

# Install packages - no lockfiles created!
clay install express react lodash

# Bundle for production
clay bundle --minify --output dist/app.js

# Start development server
clay dev --port 3000
```

## Core Commands

```bash
# Package Management
clay install [packages...]              # Install packages
clay install --dev [packages...]        # Install as dev dependencies
clay uninstall <package>                 # Remove packages
clay list                               # List installed packages

# Development
clay bundle [--output] [--minify]       # Bundle application
clay dev [--port] [--host]              # Start dev server
clay run [script]                       # Run package.json scripts

# Workspace Management
clay workspace list                     # List all workspaces
clay workspace add <name>               # Add new workspace
clay workspace run <script>             # Run script in workspaces

# Content Store
clay store stats                        # Show deduplication statistics
clay store cleanup                      # Clean unused packages
clay cache clear                       # Clear package cache
```

## Performance Benchmarks

Real-world benchmarks on identical hardware (v0.1.1+):

| Scenario | Clay | Bun | Advantage |
|----------|------|-----|-----------|
| Single package (cold) | ~150ms | ~300ms | **Clay faster** |
| Single package (warm) | ~50ms | ~50ms | **Tied** |
| Multi-package (cold) | ~950ms | ~950ms | **Tied** |  
| Multi-package (warm) | ~950ms | ~50ms | Bun faster |
| **Project cleanliness** | **No lockfiles** | **Lockfile required** | **Clay wins** |
| **Storage efficiency** | **Global dedup** | **Per-project cache** | **Clay wins** |

**Clay's advantage:** Competitive speed with revolutionary lockfile-free architecture.

## The Content Store Advantage

Clay delivers unique benefits with competitive performance:

- **Zero lockfile conflicts** in team environments
- **Perfect dependency deduplication** across all projects  
- **Guaranteed reproducibility** without file management
- **Cleaner repositories** with no lockfile noise
- **High-performance HTTP/2 client** with optimized concurrency

Clay is feature-complete with ongoing improvements to the content-addressable architecture.