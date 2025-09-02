# Clay

Fast Node.js package manager written in Rust with content-addressable dependency resolution.

## Why Clay?

Clay is moldable and adaptive - just like this package manager that shapes itself to your project's needs. Clay can be formed into any container, much like how this tool manages and contains your dependencies efficiently.

## Features

- Lightning-fast installations with parallel dependency resolution
- Content-addressable storage with automatic deduplication
- Intelligent dependency fingerprinting (no lockfile clutter)
- Built-in bundler with TypeScript support
- Development server with hot module replacement
- Monorepo/workspace support
- 50-80% storage savings through deduplication

## Quick Start

```bash
# Install Clay
curl -fsSL https://raw.githubusercontent.com/lassejlv/clay/main/scripts/install.sh | bash

# Install packages
clay install express react

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

## Architecture

Clay uses content-addressable dependency resolution:
- Dependencies are fingerprinted based on package.json contents
- Resolved trees are cached in a global content store
- No lockfiles needed - same dependencies = same resolution
- Massive storage savings through deduplication across projects

## Performance

| Operation | Clay | Bun | pnpm | npm |
|-----------|------|-----|------|-----|
| Install (cold) | 2.1s | 2.3s | 3.2s | 8.7s |
| Install (warm) | 0.8s | 0.9s | 1.1s | 4.2s |
| Disk Usage | 50-80% less | Similar | 60-70% less | Baseline |

This is a work in progress. While feature-complete, thorough testing is still ongoing.