#!/bin/bash

# Clay Package Manager Uninstallation Script
# This script removes Clay package manager from your system

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
BINARY_NAME="clay"
INSTALL_DIR="$HOME/.local/bin"
CACHE_DIR="$HOME/.cache/clay"
CONFIG_DIR="$HOME/.config/clay"

# Function to print colored output
print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to check if command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Function to prompt user for confirmation
confirm() {
    local prompt="$1"
    local response

    while true; do
        echo -n -e "${YELLOW}$prompt [y/N]:${NC} "
        read -r response
        case "$response" in
            [yY]|[yY][eE][sS])
                return 0
                ;;
            [nN]|[nN][oO]|"")
                return 1
                ;;
            *)
                echo "Please answer yes or no."
                ;;
        esac
    done
}

# Function to remove binary
remove_binary() {
    local binary_path="$INSTALL_DIR/$BINARY_NAME"

    if [ -f "$binary_path" ]; then
        print_info "Removing Clay binary from $binary_path"
        rm -f "$binary_path"
        print_success "Binary removed successfully"
    else
        print_warning "Clay binary not found at $binary_path"
    fi

    # Check if clay is still available in PATH (might be installed elsewhere)
    if command_exists clay; then
        local clay_location=$(which clay)
        print_warning "Clay is still available in PATH at: $clay_location"
        print_info "You may need to remove it manually if it was installed elsewhere"
    fi
}

# Function to remove cache
remove_cache() {
    if [ -d "$CACHE_DIR" ]; then
        local cache_size=$(du -sh "$CACHE_DIR" 2>/dev/null | cut -f1 || echo "unknown")

        if confirm "Remove Clay cache directory ($CACHE_DIR, size: $cache_size)?"; then
            print_info "Removing cache directory..."
            rm -rf "$CACHE_DIR"
            print_success "Cache directory removed"
        else
            print_info "Cache directory preserved"
        fi
    else
        print_info "No cache directory found"
    fi
}

# Function to remove config
remove_config() {
    if [ -d "$CONFIG_DIR" ]; then
        if confirm "Remove Clay configuration directory ($CONFIG_DIR)?"; then
            print_info "Removing configuration directory..."
            rm -rf "$CONFIG_DIR"
            print_success "Configuration directory removed"
        else
            print_info "Configuration directory preserved"
        fi
    else
        print_info "No configuration directory found"
    fi
}

# Function to remove PATH entries
remove_from_path() {
    local shell_profiles=(
        "$HOME/.bashrc"
        "$HOME/.bash_profile"
        "$HOME/.zshrc"
        "$HOME/.config/fish/config.fish"
    )

    local found_entries=false

    for profile in "${shell_profiles[@]}"; do
        if [ -f "$profile" ] && grep -q "$INSTALL_DIR" "$profile"; then
            found_entries=true

            if confirm "Remove Clay PATH entry from $profile?"; then
                print_info "Removing PATH entry from $profile"

                # Create a backup
                cp "$profile" "${profile}.clay-backup"

                # Remove Clay-related lines
                sed -i.tmp '/# Added by Clay installer/,/export PATH.*clay/d' "$profile" 2>/dev/null || {
                    # Fallback for systems where sed -i works differently
                    grep -v "clay" "$profile" > "${profile}.tmp" && mv "${profile}.tmp" "$profile"
                }

                print_success "PATH entry removed (backup saved as ${profile}.clay-backup)"
            fi
        fi
    done

    if [ "$found_entries" = false ]; then
        print_info "No Clay PATH entries found in shell profiles"
    fi
}

# Function to clean up any remaining Clay files
cleanup_remaining() {
    print_info "Checking for any remaining Clay files..."

    # Look for any clay-related files in common locations
    local locations=(
        "$HOME/.clay"
        "$HOME/.clay-cache"
        "/tmp/clay-*"
        "/usr/local/bin/clay"
        "/usr/bin/clay"
    )

    for location in "${locations[@]}"; do
        if [ -e $location ]; then
            print_warning "Found remaining Clay files at: $location"
            if confirm "Remove $location?"; then
                rm -rf $location
                print_success "Removed $location"
            fi
        fi
    done
}

# Function to show final status
show_final_status() {
    echo ""
    print_info "Uninstallation Summary:"
    echo "  • Binary: $([ -f "$INSTALL_DIR/$BINARY_NAME" ] && echo "❌ Still present" || echo "✅ Removed")"
    echo "  • Cache: $([ -d "$CACHE_DIR" ] && echo "❌ Still present" || echo "✅ Removed")"
    echo "  • Config: $([ -d "$CONFIG_DIR" ] && echo "❌ Still present" || echo "✅ Removed")"
    echo "  • PATH: $(command_exists clay && echo "❌ Still in PATH" || echo "✅ Removed from PATH")"
    echo ""

    if command_exists clay; then
        print_warning "Clay is still available in your PATH. You may need to restart your shell."
        print_info "If Clay persists, check: $(which clay)"
    else
        print_success "Clay has been completely removed from your system"
    fi
}

# Main uninstallation function
main() {
    echo -e "${RED}"
    echo "  ████████╗██╗      █████╗ ██╗   ██╗"
    echo "  ██╔═════╝██║     ██╔══██╗╚██╗ ██╔╝"
    echo "  ██║      ██║     ███████║ ╚████╔╝ "
    echo "  ██║      ██║     ██╔══██║  ╚██╔╝  "
    echo "  ╚███████╗███████╗██║  ██║   ██║   "
    echo "   ╚══════╝╚══════╝╚═╝  ╚═╝   ╚═╝   "
    echo -e "${NC}"
    echo "  Clay Package Manager Uninstaller"
    echo ""

    # Check if Clay is installed
    if [ ! -f "$INSTALL_DIR/$BINARY_NAME" ] && ! command_exists clay; then
        print_warning "Clay doesn't appear to be installed on this system"
        if ! confirm "Continue with cleanup anyway?"; then
            print_info "Uninstallation cancelled"
            exit 0
        fi
    fi

    print_warning "This will remove Clay package manager from your system"

    if [ "$1" != "--force" ] && [ "$1" != "-f" ]; then
        if ! confirm "Are you sure you want to continue?"; then
            print_info "Uninstallation cancelled"
            exit 0
        fi
    fi

    echo ""
    print_info "Starting Clay uninstallation..."

    # Remove components
    remove_binary
    remove_cache
    remove_config
    remove_from_path
    cleanup_remaining

    show_final_status

    echo ""
    print_success "Clay uninstallation completed!"
    print_info "Thank you for trying Clay! 🙏"
}

# Handle script arguments
case "${1:-}" in
    --help|-h)
        echo "Clay Package Manager Uninstallation Script"
        echo ""
        echo "Usage: $0 [OPTIONS]"
        echo ""
        echo "Options:"
        echo "  --help, -h     Show this help message"
        echo "  --force, -f    Skip confirmation prompts"
        echo "  --keep-cache   Keep cache directory"
        echo "  --keep-config  Keep configuration directory"
        echo ""
        echo "This script will:"
        echo "  1. Remove Clay binary"
        echo "  2. Remove cache and configuration (with confirmation)"
        echo "  3. Remove PATH entries from shell profiles"
        echo "  4. Clean up any remaining Clay files"
        exit 0
        ;;
    --keep-cache)
        KEEP_CACHE=true
        ;;
    --keep-config)
        KEEP_CONFIG=true
        ;;
esac

# Override cache/config removal if keep flags are set
if [ "$KEEP_CACHE" = true ]; then
    remove_cache() {
        print_info "Keeping cache directory (--keep-cache specified)"
    }
fi

if [ "$KEEP_CONFIG" = true ]; then
    remove_config() {
        print_info "Keeping configuration directory (--keep-config specified)"
    }
fi

# Run main uninstallation
main "$@"
