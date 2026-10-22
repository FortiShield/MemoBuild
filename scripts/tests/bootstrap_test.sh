#!/usr/bin/env bash
#
# Smoke tests for the MemoBuild bootstrap scripts.
#
# Usage:
#   scripts/tests/bootstrap_test.sh [--fast]
#
#   --fast   Skip the container test, only run the host level checks.
#
# Environment:
#   BOOTSTRAP_TEST_CONTAINER_IMAGE   Image used for the container test
#                                    (default: ubuntu:24.04).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BOOTSTRAP="${REPO_ROOT}/scripts/bootstrap"
SUDO_HELPER="${REPO_ROOT}/scripts/memobuild_sudo"

CONTAINER_IMAGE="${BOOTSTRAP_TEST_CONTAINER_IMAGE:-ubuntu:24.04}"
RUN_CONTAINER=1

for arg in "$@"; do
    case "${arg}" in
        --fast)
            RUN_CONTAINER=0
            ;;
        -h | --help)
            printf 'Usage: %s [--fast]\n' "$0"
            exit 0
            ;;
        *)
            printf 'error: unknown argument %s\n' "${arg}" >&2
            exit 64
            ;;
    esac
done

# The bootstrap of the platform this test runs on.
case "$(uname -s)" in
    Darwin) PLATFORM_BOOTSTRAP="${REPO_ROOT}/scripts/macos/bootstrap" ;;
    Linux) PLATFORM_BOOTSTRAP="${REPO_ROOT}/scripts/linux/bootstrap" ;;
    *) PLATFORM_BOOTSTRAP="" ;;
esac

PASSED=0
FAILED=0

pass() {
    printf '\033[0;32mok\033[0m   %s\n' "$1"
    PASSED=$((PASSED + 1))
}

fail() {
    printf '\033[0;31mFAIL\033[0m %s\n' "$1"
    FAILED=$((FAILED + 1))
}

skip() {
    printf '\033[0;33mskip\033[0m %s\n' "$1"
}

expect_exit() {
    local expected="$1"
    shift
    local output

    set +e
    output="$("$@" 2>&1)"
    local status=$?
    set -e

    if [[ "${status}" -eq "${expected}" ]]; then
        return 0
    fi

    printf '%s\n' "${output}" >&2
    return 1
}

# ---------------------------------------------------------------------------
# Host level checks
# ---------------------------------------------------------------------------

test_help() {
    local output
    output="$("${BOOTSTRAP}" --help 2>&1)"
    if [[ "${output}" == *"Usage: ./scripts/bootstrap"* ]]; then
        pass "--help prints the usage banner"
    else
        fail "--help prints the usage banner"
    fi
}

test_unknown_argument() {
    if expect_exit 64 "${BOOTSTRAP}" --definitely-not-an-option; then
        pass "unknown arguments are rejected with exit code 64"
    else
        fail "unknown arguments are rejected with exit code 64"
    fi
}

test_dry_run_is_side_effect_free() {
    # Record whether git-lfs is available before the dry run.
    before_git_lfs_available=0
    if command -v git-lfs >/dev/null 2>&1; then
        before_git_lfs_available=1
    fi

    if expect_exit 0 env MEMOBUILD_DRY_RUN=1 "${BOOTSTRAP}" --dry-run; then
        # After dry-run, git-lfs should still be available only if it was before.
        if command -v git-lfs >/dev/null 2>&1; then
            if [[ "${before_git_lfs_available}" -eq 1 ]]; then
                pass "--dry-run is side-effect free (git-lfs still available)"
            else
                fail "--dry-run left git-lfs installed (was not before)"
            fi
        else
            if [[ "${before_git_lfs_available}" -eq 0 ]]; then
                pass "--dry-run is side-effect free (git-lfs still unavailable)"
            else
                fail "--dry-run removed git-lfs (was available before)"
            fi
        fi
    else
        fail "--dry-run completes without errors"
    fi
}

test_check_mode() {
    if ! command -v cargo >/dev/null 2>&1; then
        skip "--check (cargo is not installed on this host)"
        return
    fi
    if expect_exit 0 "${BOOTSTRAP}" --check; then
        pass "--check verifies the environment"
    else
        fail "--check verifies the environment"
    fi
}

test_platform_bootstrap_runs_standalone() {
    if [[ -z "${PLATFORM_BOOTSTRAP}" ]]; then
        skip "standalone platform bootstrap (unsupported platform)"
        return
    fi

    # The platform bootstrap has to work on its own too, not only through the
    # wrapper in scripts/bootstrap.
    # Verify dry-run is side-effect free by checking git-lfs availability.
    before_git_lfs_available=0
    if command -v git-lfs >/dev/null 2>&1; then
        before_git_lfs_available=1
    fi

    if expect_exit 0 "${PLATFORM_BOOTSTRAP}" --dry-run; then
        if command -v git-lfs >/dev/null 2>&1; then
            if [[ "${before_git_lfs_available}" -eq 1 ]]; then
                pass "the platform bootstrap runs standalone in dry-run mode (git-lfs still available)"
            else
                fail "the platform bootstrap left git-lfs installed (was not before)"
            fi
        else
            if [[ "${before_git_lfs_available}" -eq 0 ]]; then
                pass "the platform bootstrap runs standalone in dry-run mode (git-lfs still unavailable)"
            else
                fail "the platform bootstrap removed git-lfs (was available before)"
            fi
        fi
    else
        fail "the platform bootstrap runs standalone in dry-run mode"
    fi
}

test_platform_bootstrap_rejects_unknown_arguments() {
    if [[ -z "${PLATFORM_BOOTSTRAP}" ]]; then
        skip "platform bootstrap argument handling (unsupported platform)"
        return
    fi

    if expect_exit 64 "${PLATFORM_BOOTSTRAP}" --definitely-not-an-option; then
        pass "the platform bootstrap rejects unknown arguments with exit code 64"
    else
        fail "the platform bootstrap rejects unknown arguments with exit code 64"
    fi
}

test_memobuild_sudo_usage() {
    if expect_exit 64 "${SUDO_HELPER}"; then
        pass "memobuild_sudo without a command exits with 64"
    else
        fail "memobuild_sudo without a command exits with 64"
    fi

    if expect_exit 69 "${SUDO_HELPER}" definitely-not-a-command; then
        pass "memobuild_sudo with a missing command exits with 69"
    else
        fail "memobuild_sudo with a missing command exits with 69"
    fi
}

test_no_lfs_is_accepted() {
    if expect_exit 0 "${BOOTSTRAP}" --no-lfs --dry-run; then
        pass "--no-lfs is accepted"
    else
        fail "--no-lfs is accepted"
    fi
}

test_shellcheck() {
    local linter="${REPO_ROOT}/scripts/lint_shellcheck"

    if ! command -v shellcheck >/dev/null 2>&1; then
        skip "shellcheck (not installed on this host)"
        return
    fi

    if "${linter}"; then
        pass "shellcheck reports no findings"
    else
        fail "shellcheck reports no findings"
    fi
}

# ---------------------------------------------------------------------------
# Container test
# ---------------------------------------------------------------------------

test_container() {
    if ! command -v docker >/dev/null 2>&1; then
        skip "container bootstrap (docker is not available)"
        return
    fi

    if ! docker info >/dev/null 2>&1; then
        skip "container bootstrap (the docker daemon is not running)"
        return
    fi

    printf 'running the bootstrap in %s, this takes a few minutes...\n' "${CONTAINER_IMAGE}"

    local output
    set +e
    output="$(docker run --rm \
        -v "${REPO_ROOT}:/work" \
        -w /work \
        -e "MEMOBUILD_SKIP_CARGO_AUDIT=1" \
        -e "DEBIAN_FRONTEND=noninteractive" \
        "${CONTAINER_IMAGE}" \
        bash -c 'set -e
            apt-get update -qq
            apt-get install -y -qq git curl ca-certificates >/dev/null
            ./scripts/bootstrap --yes
            . "$HOME/.cargo/env"
            cargo --version
            protoc --version' 2>&1)"
    local status=$?
    set -e

    if [[ "${status}" -ne 0 ]]; then
        printf '%s\n' "${output}" >&2
        fail "the bootstrap completes on a clean ${CONTAINER_IMAGE} container"
        return
    fi

    if [[ "${output}" == *"cargo 1."* && "${output}" == *"libprotoc"* ]]; then
        pass "the bootstrap completes on a clean ${CONTAINER_IMAGE} container"
    else
        printf '%s\n' "${output}" >&2
        fail "the bootstrap completes on a clean ${CONTAINER_IMAGE} container"
    fi
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    printf 'MemoBuild bootstrap tests\n'
    printf '========================\n\n'

    test_help
    test_unknown_argument
    test_dry_run_is_side_effect_free
    test_check_mode
    test_platform_bootstrap_runs_standalone
    test_platform_bootstrap_rejects_unknown_arguments
    test_memobuild_sudo_usage
    test_no_lfs_is_accepted
    test_shellcheck

    if ((RUN_CONTAINER == 1)); then
        test_container
    else
        skip "container bootstrap (--fast)"
    fi

    printf '\n%s passed, %s failed\n' "${PASSED}" "${FAILED}"

    if ((FAILED > 0)); then
        return 1
    fi
    return 0
}

main "$@"
