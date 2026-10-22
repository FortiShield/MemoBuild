# Development Setup

This guide describes how to prepare a machine to build and contribute to MemoBuild.

## Quick start

```bash
git clone https://github.com/nrelab/MemoBuild.git
cd MemoBuild
./scripts/bootstrap
```

The bootstrap script detects the operating system, installs the toolchain that
MemoBuild needs and verifies the result. It is idempotent: anything that is
already installed is detected and left untouched.

```
Usage: ./scripts/bootstrap [options]

Options:
  -h, --help       Show this help message.
  -y, --yes        Do not ask before elevating privileges (unattended use).
  --dry-run        Print what would be done, change nothing.
  --check          Only verify the environment, install nothing.
  --no-lfs         Skip the Git LFS setup step.
```

## What gets installed

| Tool | Needed for | Notes |
| --- | --- | --- |
| Rust (>= 1.75) | building and running MemoBuild | installed with `rustup`, default profile so `rustfmt` and `clippy` are available |
| C toolchain | the bundled SQLite build (`rusqlite`) | `build-essential` on Debian/Ubuntu |
| `pkg-config` | native dependency detection | |
| `protoc` | the `containerd` and `remote-exec` features | required by `--all-features` builds |
| `git-lfs` | large file storage | only used when the checkout tracks LFS objects |
| `cargo-audit` | `scripts/security-audit.sh` | optional, uses `cargo binstall` when available and compiles from source otherwise |
| Docker | building the desktop extension | optional, install Docker Desktop |

Optional extras are opt-in through environment variables:

| Variable | Effect |
| --- | --- |
| `MEMOBUILD_INSTALL_DOCKER=1` | install Docker from the official apt repository (Linux) |
| `MEMOBUILD_INSTALL_LINUXDEPLOY=1` | download `linuxdeploy` into `./dist` (Linux) |
| `MEMOBUILD_SKIP_CARGO_AUDIT=1` | do not install `cargo-audit` |
| `MEMOBUILD_SKIP_SUDO_PROMPT=1` | never ask before elevating privileges |
| `MEMOBUILD_DRY_RUN=1` | same as `--dry-run` |
| `MEMOBUILD_CHECK_ONLY=1` | same as `--check` |
| `NO_COLOR=1` | disable ANSI colours |

## Platform notes

The platform bootstraps (`scripts/linux/bootstrap`, `scripts/macos/bootstrap` and
`scripts/windows/bootstrap.ps1`) can also be run on their own and accept the
same options as `scripts/bootstrap`: `-y`, `--dry-run` and `--check`.

### Linux

Supports `apt` (Debian/Ubuntu), `dnf` (Fedora/RHEL) and `pacman` (Arch). Steps
that need root privileges are run through `scripts/memobuild_sudo`, which asks
for confirmation before elevating and never blocks on a password in a
non-interactive shell.

### macOS

Requires [Homebrew](https://brew.sh). The script refuses to run as root because
Homebrew does not support it. It makes sure the Xcode command line tools are
installed, switches the active developer directory to `/Applications/Xcode.app`
when a full Xcode is present and adds the `aarch64-apple-darwin` Rust target
that the release workflow builds.

### Windows

Requires `winget` (recommended) or `chocolatey`. Run it from a PowerShell prompt:

```powershell
pwsh -File scripts/windows/bootstrap.ps1
```

If the execution policy blocks the script:

```powershell
pwsh -ExecutionPolicy Bypass -File scripts/windows/bootstrap.ps1
```

## Verifying a machine

```bash
./scripts/bootstrap --check
```

This prints the state of every tool the project depends on and exits with a
non-zero status when a required tool is missing. It never changes anything.

## Testing the bootstrap scripts

```bash
./scripts/tests/bootstrap_test.sh --fast   # host level checks only
./scripts/tests/bootstrap_test.sh          # also runs a clean container test
```

The container test starts an `ubuntu:24.04` container, runs the bootstrap from
scratch and verifies that `cargo` and `protoc` are available afterwards. It
needs Docker and network access.

## Linting shell scripts

```bash
./scripts/lint_shellcheck
```

Every shell script in the repository is discovered by its shebang and checked
with ShellCheck, so files without a `.sh` extension are covered as well.

| Option | Effect |
| --- | --- |
| `-ci`, `--ci` | fail when ShellCheck is not installed (the CI job uses this) |
| `-s`, `--severity LEVEL` | minimum severity to report: `style`, `warning` or `info` |
| `-h`, `--help` | show the usage banner |

Exit codes: `0` no findings, `1` findings were reported, `2` invalid usage,
`3` ShellCheck is required but not installed.

## After the bootstrap

```bash
cargo build
cargo test
cargo run -- --help
```
