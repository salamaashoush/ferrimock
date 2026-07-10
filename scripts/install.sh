#!/bin/sh
# Ferrimock installer
# Usage: curl -sSf https://raw.githubusercontent.com/salamaashoush/ferrimock/main/scripts/install.sh | sh
#
# Environment variables:
#   FERRIMOCK_VERSION  - Version to install (default: latest)
#   INSTALL_DIR      - Installation directory (default: /usr/local/bin)

set -eu

REPO="salamaashoush/ferrimock"
BINARY="ferrimock"

# Colors (only if terminal supports it)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    CYAN='\033[0;36m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED='' GREEN='' YELLOW='' CYAN='' BOLD='' NC=''
fi

info()  { printf "${CYAN}info${NC}  %s\n" "$1"; }
ok()    { printf "${GREEN}ok${NC}    %s\n" "$1"; }
warn()  { printf "${YELLOW}warn${NC}  %s\n" "$1"; }
error() { printf "${RED}error${NC} %s\n" "$1" >&2; exit 1; }

# Detect OS
detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "unknown-linux-musl" ;;
        Darwin*) echo "apple-darwin" ;;
        MINGW*|MSYS*|CYGWIN*) echo "pc-windows-msvc" ;;
        *) error "Unsupported operating system: $(uname -s)" ;;
    esac
}

# Detect architecture
detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)   echo "x86_64" ;;
        aarch64|arm64)  echo "aarch64" ;;
        *) error "Unsupported architecture: $(uname -m)" ;;
    esac
}

# Detect download tool
detect_downloader() {
    if command -v curl >/dev/null 2>&1; then
        echo "curl"
    elif command -v wget >/dev/null 2>&1; then
        echo "wget"
    else
        error "Neither curl nor wget found. Please install one of them."
    fi
}

# Download a URL to a file
download() {
    url="$1"
    output="$2"
    downloader="$(detect_downloader)"

    case "$downloader" in
        curl) curl --proto '=https' --tlsv1.2 -sSfL "$url" -o "$output" ;;
        wget) wget -q "$url" -O "$output" ;;
    esac
}

# Get latest version from GitHub
get_latest_version() {
    downloader="$(detect_downloader)"
    case "$downloader" in
        curl) curl --proto '=https' --tlsv1.2 -sSf "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed 's/.*"v\(.*\)".*/\1/' ;;
        wget) wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed 's/.*"v\(.*\)".*/\1/' ;;
    esac
}

main() {
    printf "\n${BOLD}Ferrimock Installer${NC}\n\n"

    # Detect platform
    OS="$(detect_os)"
    ARCH="$(detect_arch)"
    TARGET="${ARCH}-${OS}"
    info "Detected platform: ${TARGET}"

    # Determine version
    VERSION="${FERRIMOCK_VERSION:-}"
    if [ -z "$VERSION" ]; then
        info "Fetching latest version..."
        VERSION="$(get_latest_version)"
    fi

    if [ -z "$VERSION" ]; then
        error "Could not determine version. Set FERRIMOCK_VERSION or check https://github.com/${REPO}/releases"
    fi
    info "Version: v${VERSION}"

    # Determine archive format
    case "$OS" in
        *windows*) EXT="zip" ;;
        *)         EXT="tar.gz" ;;
    esac

    ARCHIVE="${BINARY}-${VERSION}-${TARGET}.${EXT}"
    URL="https://github.com/${REPO}/releases/download/v${VERSION}/${ARCHIVE}"
    CHECKSUM_URL="${URL}.sha256"

    # Create temp directory
    TMPDIR="$(mktemp -d)"
    trap 'rm -rf "$TMPDIR"' EXIT

    # Download archive
    info "Downloading ${ARCHIVE}..."
    download "$URL" "${TMPDIR}/${ARCHIVE}"
    ok "Downloaded"

    # Verify checksum
    info "Verifying checksum..."
    download "$CHECKSUM_URL" "${TMPDIR}/${ARCHIVE}.sha256"
    cd "$TMPDIR"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c "${ARCHIVE}.sha256" >/dev/null 2>&1 || error "Checksum verification failed"
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c "${ARCHIVE}.sha256" >/dev/null 2>&1 || error "Checksum verification failed"
    else
        warn "No checksum tool found, skipping verification"
    fi
    ok "Checksum verified"

    # Extract
    info "Extracting..."
    case "$EXT" in
        tar.gz) tar xzf "${ARCHIVE}" ;;
        zip)    unzip -q "${ARCHIVE}" ;;
    esac
    ok "Extracted"

    # Install
    INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
    info "Installing to ${INSTALL_DIR}..."

    if [ -w "$INSTALL_DIR" ]; then
        mv "${BINARY}" "${INSTALL_DIR}/${BINARY}"
    else
        sudo mv "${BINARY}" "${INSTALL_DIR}/${BINARY}"
    fi
    chmod +x "${INSTALL_DIR}/${BINARY}"
    ok "Installed to ${INSTALL_DIR}/${BINARY}"

    # Verify
    INSTALLED_VERSION="$("${INSTALL_DIR}/${BINARY}" --version 2>/dev/null || echo "unknown")"
    ok "ferrimock ${INSTALLED_VERSION} is ready"

    # Check PATH
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ;;
        *) warn "${INSTALL_DIR} is not in your PATH. Add it with: export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
    esac

    printf "\n${BOLD}Run 'ferrimock --help' to get started.${NC}\n\n"
}

main
