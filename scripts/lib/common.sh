#!/usr/bin/env bash
#
# Shared helpers for the MemoBuild bootstrap scripts.
#
# This file is meant to be sourced, never executed directly:
#
#   source "$(dirname "${BASH_SOURCE[0]}")/lib/common.sh"
#
# Environment variables honoured by every helper:
#
#   MEMOBUILD_REPO_ROOT          Absolute path to the repository root.
#   MEMOBUILD_MIN_RUST_VERSION   Minimum acceptable rustc version (default 1.75.0).
#   MEMOBUILD_DRY_RUN=1          Print what would happen, change nothing.
#   MEMOBUILD_CHECK_ONLY=1       Only verify the environment, install nothing.
#   MEMOBUILD_SKIP_SUDO_PROMPT=1 Do not ask before elevating privileges.
#   MEMOBUILD_SKIP_CARGO_AUDIT=1 Do not install cargo-audit.
#   NO_COLOR                     Disable ANSI colours.

if [[ -n "${MEMOBUILD_COMMON_SOURCED:-}" ]]; then
    return 0
fi
MEMOBUILD_COMMON_SOURCED=1

# Absolute path of this file. Resolved once, while the sourcing script is still
# running from the directory it was started in, so that later `cd` calls cannot
# break the lookup of the repository root.
MB_COMMON_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ---------------------------------------------------------------------------
# Output helpers
# ---------------------------------------------------------------------------

if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
    MB_RESET=$'\033[0m'
    MB_BOLD=$'\033[1m'
    MB_RED=$'\033[0;31m'
    MB_GREEN=$'\033[0;32m'
    MB_YELLOW=$'\033[0;33m'
    MB_BLUE=$'\033[0;34m'
    MB_CYAN=$'\033[0;36m'
else
    MB_RESET=""
    MB_BOLD=""
    MB_RED=""
    MB_GREEN=""
    MB_YELLOW=""
    MB_BLUE=""
    MB_CYAN=""
fi

mb_log() { printf '%s\n' "$*"; }
mb_info() { printf '%s==>%s %s\n' "${MB_BLUE}" "${MB_RESET}" "$*"; }
mb_step() { printf '%s==>%s %s\n' "${MB_CYAN}" "${MB_RESET}" "$*"; }
mb_ok() { printf '%s  ok%s %s\n' "${MB_GREEN}" "${MB_RESET}" "$*"; }
mb_skip() { printf '%sskipped%s %s\n' "${MB_YELLOW}" "${MB_RESET}" "$*"; }
mb_warn() { printf '%swarning:%s %s\n' "${MB_YELLOW}" "${MB_RESET}" "$*" >&2; }
mb_error() { printf '%serror:%s %s\n' "${MB_RED}" "${MB_RESET}" "$*" >&2; }

mb_title() {
    local title="$*"
    local underline=""
    local i
    for ((i = 0; i < ${#title}; i++)); do
        underline+="-"
    done
    printf '\n%s%s%s\n' "${MB_BOLD}" "${title}" "${MB_RESET}"
    printf '%s\n' "${underline}"
}

# ---------------------------------------------------------------------------
# Mode helpers
# ---------------------------------------------------------------------------

mb_dry_run() { [[ "${MEMOBUILD_DRY_RUN:-0}" == "1" ]]; }
mb_check_only() { [[ "${MEMOBUILD_CHECK_ONLY:-0}" == "1" ]]; }

# Run a command unless dry-run mode is active.
mb_run() {
    if mb_dry_run; then
        printf '%s[dry-run]%s %s\n' "${MB_BLUE}" "${MB_RESET}" "$*"
        return 0
    fi
    "$@"
}

# ---------------------------------------------------------------------------
# Option parsing
# ---------------------------------------------------------------------------

# Print the usage banner of a platform bootstrap script.
mb_bootstrap_usage() {
    cat <<EOF
Usage: $0 [options]

Install the toolchain required to build MemoBuild. The script is idempotent:
tools that are already present are detected and skipped.

Options:
  -y, --yes    Do not ask before elevating privileges (unattended use).
  --dry-run    Print what would be done, change nothing.
  --check      Only verify the environment, install nothing.
  -h, --help   Show this help message.
EOF
}

# Parse the options shared by the platform bootstrap scripts and turn them into
# the environment variables the helpers read. Returns 64 on an unknown option.
mb_parse_bootstrap_args() {
    while (($# > 0)); do
        case "$1" in
            -y | --yes)
                export MEMOBUILD_SKIP_SUDO_PROMPT=1
                ;;
            --dry-run)
                export MEMOBUILD_DRY_RUN=1
                ;;
            --check)
                export MEMOBUILD_CHECK_ONLY=1
                ;;
            -h | --help)
                mb_bootstrap_usage
                exit 0
                ;;
            *)
                mb_error "unknown argument $1"
                return 64
                ;;
        esac
        shift
    done
    return 0
}

# ---------------------------------------------------------------------------
# Command helpers
# ---------------------------------------------------------------------------

mb_have_cmd() { command -v "$1" >/dev/null 2>&1; }

mb_require_cmd() {
    local cmd="$1"
    local hint="${2:-}"
    if ! mb_have_cmd "${cmd}"; then
        mb_error "required command '${cmd}' was not found${hint:+ (${hint})}"
        return 1
    fi
    return 0
}

# Compare two dotted numeric versions. Returns 0 when $1 >= $2.
mb_version_ge() {
    local have="$1"
    local want="$2"
    local -a h=() w=()
    local i hv wv

    IFS='.' read -r -a h <<<"${have}"
    IFS='.' read -r -a w <<<"${want}"

    for ((i = 0; i < ${#w[@]}; i++)); do
        hv="${h[i]:-0}"
        wv="${w[i]:-0}"
        hv="${hv%%[^0-9]*}"
        wv="${wv%%[^0-9]*}"
        hv="${hv:-0}"
        wv="${wv:-0}"
        if ((10#$hv > 10#$wv)); then
            return 0
        fi
        if ((10#$hv < 10#$wv)); then
            return 1
        fi
    done
    return 0
}

mb_rust_version() {
    rustc --version 2>/dev/null | awk '{print $2}'
}

# Ask the user a yes/no question. Non-interactive shells continue
# automatically; MEMOBUILD_SKIP_SUDO_PROMPT=1 always answers yes.
mb_confirm() {
    local question="$1"
    local reply

    if [[ "${MEMOBUILD_SKIP_SUDO_PROMPT:-0}" == "1" ]]; then
        return 0
    fi

    if [[ ! -t 0 ]]; then
        mb_warn "non-interactive shell, continuing without confirmation: ${question}"
        return 0
    fi

    printf '%s [y/N] ' "${question}"
    read -r reply || reply=""
    case "${reply}" in
        [yY] | [yY][eE][sS]) return 0 ;;
        *) return 1 ;;
    esac
}

# ---------------------------------------------------------------------------
# Repository helpers
# ---------------------------------------------------------------------------

mb_repo_root() {
    if [[ -n "${MEMOBUILD_REPO_ROOT:-}" ]]; then
        printf '%s\n' "${MEMOBUILD_REPO_ROOT}"
        return 0
    fi
    cd "${MB_COMMON_DIR}/../.." && pwd
}

# True when the checkout actually tracks Git LFS objects. Repositories without
# a .gitattributes that references the LFS filter do not need git-lfs at all.
mb_repo_uses_lfs() {
    local root
    root="$(mb_repo_root)"
    [[ -f "${root}/.gitattributes" ]] || return 1
    grep -qE 'filter=lfs' "${root}/.gitattributes" 2>/dev/null
}

# ---------------------------------------------------------------------------
# Toolchain steps
# ---------------------------------------------------------------------------

mb_ensure_git_lfs() {
    if ! mb_repo_uses_lfs; then
        mb_skip "git-lfs (this checkout does not track LFS objects)"
        return 0
    fi

    if mb_have_cmd git-lfs; then
        if mb_check_only; then
            mb_ok "git-lfs $(git lfs version 2>/dev/null | awk '{print $1}')"
            return 0
        fi
        if mb_dry_run; then
            mb_skip "git-lfs (already installed)"
            return 0
        fi
        git lfs install --local >/dev/null
        git lfs pull || mb_warn "git lfs pull failed; continuing"
        mb_ok "git-lfs $(git lfs version 2>/dev/null | awk '{print $1}')"
        return 0
    fi

    if mb_dry_run; then
        mb_skip "would install git-lfs"
        return 0
    fi

    if mb_check_only; then
        # The environment verification reports the missing tool with the full
        # list of problems, so do not abort the run here.
        mb_warn "git-lfs is missing"
        return 0
    fi

    mb_error "git-lfs is required by this checkout but is not installed"
    case "$(uname -s)" in
        Darwin) mb_error "install it with: brew install git-lfs" ;;
        Linux) mb_error "install it with: sudo apt-get install -y git-lfs" ;;
        *) mb_error "see https://git-lfs.com for install instructions" ;;
    esac
    return 1
}

# Install or validate the Rust toolchain. Delegates to scripts/install_rust.
mb_ensure_rust() {
    local minimum="${MEMOBUILD_MIN_RUST_VERSION:-1.75.0}"
    local version

    if mb_have_cmd cargo && mb_have_cmd rustc; then
        version="$(mb_rust_version)"
        if mb_version_ge "${version}" "${minimum}"; then
            mb_ok "rustc ${version}"
            return 0
        fi
        mb_warn "rustc ${version} is older than the required ${minimum}"
    else
        mb_warn "no Rust toolchain found, installing one via rustup"
    fi

    if mb_dry_run; then
        mb_skip "would install the Rust toolchain (>= ${minimum})"
        return 0
    fi

    local installer
    installer="$(mb_repo_root)/scripts/install_rust"
    if [[ ! -f "${installer}" ]]; then
        mb_error "missing Rust installer: ${installer}"
        return 1
    fi

    bash "${installer}"

    # install_rust runs in its own shell, so the environment it set up has to
    # be imported here as well.
    if [[ -f "${HOME}/.cargo/env" ]]; then
        # shellcheck disable=SC1091
        source "${HOME}/.cargo/env"
    fi

    if ! mb_have_cmd cargo; then
        mb_error "cargo is still not on PATH after installing the toolchain"
        mb_error "open a new shell (or source \$HOME/.cargo/env) and re-run the bootstrap"
        return 1
    fi

    version="$(mb_rust_version)"
    if ! mb_version_ge "${version}" "${minimum}"; then
        mb_error "rustc ${version} is still older than the required ${minimum}"
        return 1
    fi

    mb_ok "rustc ${version}"
}

# Install cargo-audit, the dependency scanner used by scripts/security-audit.sh.
mb_ensure_cargo_audit() {
    if mb_have_cmd cargo-audit; then
        mb_ok "cargo-audit already installed"
        return 0
    fi

    if [[ "${MEMOBUILD_SKIP_CARGO_AUDIT:-0}" == "1" ]]; then
        mb_skip "cargo-audit (MEMOBUILD_SKIP_CARGO_AUDIT=1)"
        return 0
    fi

    if mb_dry_run; then
        mb_skip "would install cargo-audit"
        return 0
    fi

    mb_step "installing cargo-audit (this compiles from source and takes a few minutes)"

    # cargo-binstall ships prebuilt binaries and takes seconds, compiling
    # cargo-audit from source takes minutes.
    if mb_have_cmd cargo-binstall; then
        mb_step "installing the prebuilt cargo-audit binary with cargo-binstall"
        if cargo binstall --no-confirm cargo-audit; then
            mb_ok "cargo-audit installed"
            return 0
        fi
        mb_warn "cargo binstall failed, falling back to cargo install"
    fi

    if ! cargo install cargo-audit --locked; then
        mb_warn "cargo install cargo-audit --locked failed, retrying without --locked"
        cargo install cargo-audit || {
            mb_warn "could not install cargo-audit; install it later with 'cargo install cargo-audit'"
            return 0
        }
    fi
    mb_ok "cargo-audit installed"
}

# ---------------------------------------------------------------------------
# Verification
# ---------------------------------------------------------------------------

# Verify that the environment can build MemoBuild. Returns the number of
# hard failures (0 when everything required is present).
mb_verify_environment() {
    local failures=0
    local root
    root="$(mb_repo_root)"

    mb_title "Verifying the MemoBuild toolchain"

    if mb_have_cmd cargo; then
        mb_ok "cargo           $(cargo --version)"
    else
        mb_error "cargo           missing (required)"
        failures=$((failures + 1))
    fi

    if mb_have_cmd rustc; then
        local version minimum
        version="$(mb_rust_version)"
        minimum="${MEMOBUILD_MIN_RUST_VERSION:-1.75.0}"
        if mb_version_ge "${version}" "${minimum}"; then
            mb_ok "rustc           ${version}"
        else
            mb_error "rustc           ${version} (need >= ${minimum})"
            failures=$((failures + 1))
        fi
    else
        mb_error "rustc           missing (required)"
        failures=$((failures + 1))
    fi

    if mb_have_cmd cc || mb_have_cmd gcc || mb_have_cmd clang; then
        mb_ok "c compiler      available"
    else
        mb_error "c compiler      missing (required by the bundled SQLite build)"
        failures=$((failures + 1))
    fi

    if mb_have_cmd protoc; then
        mb_ok "protoc          $(protoc --version)"
    else
        mb_warn "protoc          missing (only needed for --all-features builds)"
    fi

    if mb_have_cmd pkg-config; then
        mb_ok "pkg-config      available"
    else
        mb_warn "pkg-config      missing (some native crates may fail to build)"
    fi

    if mb_have_cmd cargo-audit; then
        mb_ok "cargo-audit     installed"
    else
        mb_warn "cargo-audit     missing (optional, used by scripts/security-audit.sh)"
    fi

    if mb_have_cmd git-lfs; then
        mb_ok "git-lfs         $(git lfs version 2>/dev/null | awk '{print $1}')"
    elif mb_repo_uses_lfs; then
        mb_error "git-lfs         missing (required by this checkout)"
        failures=$((failures + 1))
    else
        mb_warn "git-lfs         missing (not used by this checkout)"
    fi

    if mb_have_cmd docker; then
        mb_ok "docker          $(docker --version 2>/dev/null)"
    else
        mb_warn "docker          missing (optional, needed to build the extension)"
    fi

    if mb_have_cmd node; then
        mb_ok "node            $(node --version 2>/dev/null)"
    else
        mb_warn "node            missing (optional, needed for the extension UI)"
    fi

    if [[ -f "${root}/Cargo.toml" ]]; then
        if (cd "${root}" && cargo metadata --offline --no-deps --format-version 1 >/dev/null 2>&1); then
            mb_ok "manifest        Cargo.toml resolves"
        else
            mb_error "manifest        Cargo.toml does not resolve"
            failures=$((failures + 1))
        fi
    fi

    if ((failures == 0)); then
        mb_ok "environment is ready"
    else
        mb_error "${failures} required item(s) are missing"
    fi

    return "${failures}"
}

mb_print_path_hint() {
    if ! mb_have_cmd cargo && [[ -f "${HOME}/.cargo/env" ]]; then
        mb_warn "cargo was installed during this run but is not on the PATH of the current shell"
        mb_log "  source \"${HOME}/.cargo/env\"   # or open a new shell"
    fi
}

mb_print_next_steps() {
    mb_title "Next steps"
    mb_log "  cargo build"
    mb_log "  cargo test"
    mb_log "  cargo run -- --help"
}
