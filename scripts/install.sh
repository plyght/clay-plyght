#!/bin/bash

# Clay Package Manager Installation Script
# This script installs Clay, a fast package manager for Node.js written in Rust

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
REPO_URL="https://github.com/lassejlv/clay"
BINARY_NAME="clay"
INSTALL_DIR="$HOME/.local/bin"

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

# Function to detect OS and architecture
detect_platform() {
    local os=$(uname -s | tr '[:upper:]' '[:lower:]')
    local arch=$(uname -m)

    case "$arch" in
        x86_64|amd64)
            arch="x86_64"
            ;;
        arm64|aarch64)
            arch="aarch64"
            ;;
        *)
            print_error "Unsupported architecture: $arch"
            exit 1
            ;;
    esac

    case "$os" in
        linux)
            PLATFORM="$arch-unknown-linux-gnu"
            ;;
        darwin)
            PLATFORM="$arch-apple-darwin"
            ;;
        *)
            print_error "Unsupported operating system: $os"
            exit 1
            ;;
    esac
}

# Function to check prerequisites
check_prerequisites() {
    print_info "Checking prerequisites..."

    # Check if Rust is installed for building from source
    if command_exists rustc && command_exists cargo; then
        RUST_AVAILABLE=true
        local rust_version=$(rustc --version | cut -d' ' -f2)
        print_info "Found Rust $rust_version"
    else
        RUST_AVAILABLE=false
        print_warning "Rust not found. Will attempt to install pre-built binary if available."
    fi

    # Check if git is available
    if command_exists git; then
        GIT_AVAILABLE=true
    else
        GIT_AVAILABLE=false
        print_warning "Git not found. Some installation methods may not be available."
    fi

    # Check if curl is available
    if ! command_exists curl; then
        print_error "curl is required but not installed. Please install curl and try again."
        exit 1
    fi
}

# Function to create install directory
create_install_dir() {
    if [ ! -d "$INSTALL_DIR" ]; then
        print_info "Creating install directory: $INSTALL_DIR"
        mkdir -p "$INSTALL_DIR"
    fi
}

# Function to install from pre-built binary (if available)
install_prebuilt() {
    print_info "Attempting to download pre-built binary..."

    local download_url="${REPO_URL}/releases/latest/download/${BINARY_NAME}-${PLATFORM}"
    local temp_file="/tmp/${BINARY_NAME}"

    if curl -fsSL "$download_url" -o "$temp_file" 2>/dev/null; then
        chmod +x "$temp_file"
        mv "$temp_file" "$INSTALL_DIR/$BINARY_NAME"
        print_success "Pre-built binary installed successfully!"
        return 0
    else
        print_warning "Pre-built binary not available for your platform."
        return 1
    fi
}

# Function to build from source
build_from_source() {
    if [ "$RUST_AVAILABLE" != true ]; then
        print_error "Rust is required to build from source but is not installed."
        print_info "Please install Rust from https://rustup.rs/ and try again."
        exit 1
    fi

    if [ "$GIT_AVAILABLE" != true ]; then
        print_error "Git is required to build from source but is not installed."
        exit 1
    fi

    print_info "Building Clay from source..."

    local temp_dir="/tmp/clay-build"
    rm -rf "$temp_dir"

    print_info "Cloning repository..."
    git clone "$REPO_URL" "$temp_dir"

    cd "$temp_dir"

    print_info "Building binary (this may take a few minutes)..."
    cargo build --release

    if [ -f "target/release/$BINARY_NAME" ]; then
        cp "target/release/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
        print_success "Clay built and installed successfully!"
    else
        print_error "Build failed. Binary not found."
        exit 1
    fi

    # Clean up
    cd /
    rm -rf "$temp_dir"
}

# Function to update PATH
update_path() {
    local shell_profile=""

    # Detect shell and set appropriate profile file
    case "$SHELL" in
        */bash)
            if [ -f "$HOME/.bashrc" ]; then
                shell_profile="$HOME/.bashrc"
            elif [ -f "$HOME/.bash_profile" ]; then
                shell_profile="$HOME/.bash_profile"
            fi
            ;;
        */zsh)
            shell_profile="$HOME/.zshrc"
            ;;
        */fish)
            shell_profile="$HOME/.config/fish/config.fish"
            ;;
    esac

    # Check if install directory is already in PATH
    if echo "$PATH" | grep -q "$INSTALL_DIR"; then
        print_info "Install directory already in PATH"
        return
    fi

    if [ -n "$shell_profile" ] && [ -f "$shell_profile" ]; then
        if ! grep -q "$INSTALL_DIR" "$shell_profile"; then
            print_info "Adding $INSTALL_DIR to PATH in $shell_profile"
            echo "" >> "$shell_profile"
            echo "# Added by Clay installer" >> "$shell_profile"
            echo "export PATH=\"$INSTALL_DIR:\$PATH\"" >> "$shell_profile"
            print_warning "Please restart your shell or run: source $shell_profile"
        fi
    else
        print_warning "Could not automatically update PATH."
        print_info "Please add the following to your shell profile:"
        print_info "export PATH=\"$INSTALL_DIR:\$PATH\""
    fi
}

# Function to verify installation
verify_installation() {
    if [ -x "$INSTALL_DIR/$BINARY_NAME" ]; then
        print_success "Clay installed successfully to $INSTALL_DIR/$BINARY_NAME"

        # Try to run clay if it's in PATH
        if command_exists clay; then
            local version=$(clay --version 2>/dev/null || echo "unknown")
            print_success "Clay version: $version"
        else
            print_info "Clay installed but not yet in PATH. Please restart your shell."
        fi

        print_info ""
        print_info "Usage: clay install <package>"
        print_info "       clay --help for more options"
    else
        print_error "Installation verification failed. Binary not found at expected location."
        exit 1
    fi
}

# Main installation function
main() {
    echo -e "${BLUE}"
    echo "  ████████╗██╗      █████╗ ██╗   ██╗"
    echo "  ██╔═════╝██║     ██╔══██╗╚██╗ ██╔╝"
    echo "  ██║      ██║     ███████║ ╚████╔╝ "
    echo "  ██║      ██║     ██╔══██║  ╚██╔╝  "
    echo "  ╚███████╗███████╗██║  ██║   ██║   "
    echo "   ╚══════╝╚══════╝╚═╝  ╚═╝   ╚═╝   "
    echo -e "${NC}"
    echo "  Fast Package Manager for Node.js"
    echo ""

    print_info "Starting Clay installation..."

    detect_platform
    print_info "Detected platform: $PLATFORM"

    check_prerequisites
    create_install_dir

    # Try pre-built binary first, fall back to building from source
    if ! install_prebuilt; then
        print_info "Falling back to building from source..."
        build_from_source
    fi

    update_path
    verify_installation

    echo ""
    print_success "Clay installation completed!"
    print_info "Get started by running: clay --help"
}

# Handle script arguments
case "${1:-}" in
    --help|-h)
        echo "Clay Package Manager Installation Script"
        echo ""
        echo "Usage: $0 [OPTIONS]"
        echo ""
        echo "Options:"
        echo "  --help, -h     Show this help message"
        echo "  --source       Force build from source"
        echo "  --dir DIR      Install to custom directory (default: ~/.local/bin)"
        echo ""
        echo "This script will:"
        echo "  1. Check system requirements"
        echo "  2. Download pre-built binary or build from source"
        echo "  3. Install Clay to ~/.local/bin"
        echo "  4. Update PATH in your shell profile"
        exit 0
        ;;
    --source)
        FORCE_SOURCE=true
        ;;
    --dir)
        if [ -n "$2" ]; then
            INSTALL_DIR="$2"
            shift
        else
            print_error "--dir requires a directory argument"
            exit 1
        fi
        ;;
esac

# Run main installation
main
