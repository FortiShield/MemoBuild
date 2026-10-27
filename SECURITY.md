# Security Policy for MemoBuild

**Last Updated:** February 21, 2026  
**Status:** Active

## 1. Reporting Security Vulnerabilities

### Please DO NOT file public issues for security vulnerabilities

If you discover a security vulnerability in MemoBuild, please email **security@memobuild.dev** with:
- A clear description of the vulnerability
- Steps to reproduce (if applicable)
- Affected versions
- Suggested fix (if available)

We will acknowledge receipt within 24 hours and provide updates every 5 business days.

### Security Embargo

- Discovered vulnerabilities will be fixed in a patch release
- The patch will be released simultaneously with public disclosure
- Researchers will be credited unless they request anonymity

---

## 2. Security Best Practices

### 2.1 Dependency Management

**Current State:**
- ✅ All dependencies pinned to specific versions in Cargo.toml
- ✅ `cargo audit` run on every CI build
- ✅ Automated dependency updates via Dependabot

**Recommendations:**
- [ ] Use minimal dependencies (prefer stdlib when possible)
- [ ] Review new dependencies before update
- [ ] Use `cargo-tree` to audit dependency tree depth

### 2.2 Cryptographic Operations

**Current Usage:**
- BLAKE3 for content hashing (secure, audited algorithm)
- No cryptographic key management (TLS only)
- SHA2 for legacy compatibility (optional)

**Security Stance:**
- ✅ BLAKE3 is cryptographically secure
- ⚠️  Ensure hash outputs are never truncated for security purposes
- ⚠️  Never use output as encryption key material

### 2.3 Input Validation

**Critical Areas:**

1. **Dockerfile Parsing** (`src/docker/parser.rs`)
   - 📌 Status: Basic parsing, no strict grammar enforcement
   - Risk: Malformed instructions could cause unexpected behavior
   - Recommendation: Add fuzzing tests
   - [ ] Implement parser fuzzing with libfuzzer
   - [ ] Add bounds checking on instruction parameters

2. **Registry Authentication** (`src/export/registry.rs`)
   - 📌 Status: Bearer token stored in memory
   - Risk: Token exposed in crash dumps or memory analysis
   - Recommendation: Use secure credential storage
   - [ ] Integrate `keyring` crate for OS-level storage
   - [ ] Clear sensitive data from memory after use
   - [ ] Never log authentication tokens

3. **Hash Verification** (`src/server/mod.rs`)
   - 📌 Status: CAS verification enforced (as of fix)
   - Risk: Bypassing could lead to cache poisoning
   - Status: ✅ Now enforced with error on mismatch

### 2.4 Network Security

**HTTP Connections:**
- Remote cache uses HTTP (not HTTPS by default)
- Recommendation: Use HTTP with mutual TLS or OAuth2
- [ ] Add HTTPS support with certificate pinning option
- [ ] Implement request signing for artifact authenticity

**Build Container Execution:**
- v0.2.0 ran build commands on the host by default; its containerd backend was simulated.
- The `production/v0.2.x-hardening` branch defaults to Bubblewrap on Linux x86_64, requires working unprivileged user namespaces, and fails closed if isolation cannot be established. `unsafe-local-execution` is an explicit unsafe development-only opt-in.
- Limitations: the staged workspace has no aggregate disk quota; no cgroup or seccomp policy is installed; filesystem snapshots and OCI layers are not produced. See [`docs/production/IMPLEMENTATION_AUDIT.md`](docs/production/IMPLEMENTATION_AUDIT.md#mb-p0-001-implementation-on-productionv02x-hardening).

### 2.5 Access Control

**File Operations:**
- Local cache stored with default permissions
- Recommendation: Use restrictive permissions (0700)
- [ ] Document permission setup in deployment guide

**Server Endpoints:**
- No authentication on cache server
- Recommendation: Add API keys or mutual TLS
- [ ] Implement optional Bearer token validation
- [ ] Add rate limiting
- [ ] Add request logging for audit trail

### 2.6 Secrets Management

**Current Vulnerabilities:**
- Docker credential helpers not integrated
- Registry tokens stored as environment variables
- No secret rotation support

**Recommendations:**
```rust
// Instead of:
let token = env::var("MEMOBUILD_TOKEN")?;

// Use keyring:
let token = keyring::get("memobuild", "registry_token")?;
```

- [ ] Integrate `keyring` crate
- [ ] Support `DOCKER_CONFIG` for credentials
- [ ] Add secret rotation APIs

---

## 3. Known Security Limitations

### 3.1 Cache Poisoning Risk

**Scenario:** Attacker modifies artifact on storage device  
**Current Mitigation:** CAS verification on retrieval  
**Recommendation:** Add HMAC signatures

```rust
// Future improvement:
pub fn sign_artifact(data: &[u8], secret: &[u8]) -> String {
    // Use HMAC-SHA256 for authenticity
}
```

### 3.2 Build Container Escape

**Scenario:** Malicious Dockerfile escapes sandbox  
**v0.2.0:** No enforced isolation; host-shell execution was the default.
**MB-P0-001 branch mitigation:** Bubblewrap user/PID/IPC/UTS/network namespaces, isolated writable workspace, read-only system mounts, no capabilities, resource limits, timeout, and parent-death cleanup. Focused tests cover a protected host-file write, missing-runtime and CLI-default fail-closed behavior, network namespace separation, and timeout cleanup.
**Remaining risk:** Bubblewrap/kernel vulnerabilities, missing aggregate storage quotas and cgroup/seccomp controls, and no staged-rootfs output restoration. Do not treat this as a complete container runtime.

```dockerfile
# Operational prerequisite:
# Install bubblewrap and enable unprivileged user namespaces on Linux x86_64.
```

### 3.3 Side-Channel Attacks

**Risk:** Timing-based attacks on hash verification  
**Current Mitigation:** Direct string comparison  
**Recommendation:** Use constant-time comparison

```rust
// Use this in CAS verification:
use subtle::ConstantTimeComparison;
if !blake3::blake3(&expected, &actual, actual_hash) {
    // timing-safe comparison
}
```

---

## 4. Security Audit Checklist

### Pre-Release (v0.2.0)

- [ ] Run `cargo audit` (automated in CI)
- [ ] Run `cargo-outdated` for old dependencies
- [ ] Static analysis with LINTER
- [ ] Dependency review (check each new/updated dependency)
- [ ] Security feature verification:
  - [ ] CAS integrity enforcement
  - [ ] Token security
  - [ ] Privilege escalation prevention
  - [ ] Denial of service prevention

### After Security Incident

- [ ] Create postmortem document
- [ ] Implement preventive measures
- [ ] Update security policy
- [ ] Release patch version
- [ ] Announce in security advisory

---

## 5. Security Testing Strategy

### 5.1 Unit Tests

- Error path coverage (especially security-related)
- CAS verification enforcement
- Permission validation

### 5.2 Integration Tests

- End-to-end artifact verification
- Cache coherency under concurrent access
- Network error handling

### 5.3 Fuzzing

```bash
# Proposed fuzzing targets:
cargo fuzz parser              # Dockerfile parser
cargo fuzz registry_manifest   # OCI manifest parsing
cargo fuzz cache_operations    # Cache operations
```

### 5.4 Vulnerability Scanning

- Automated with `cargo audit` (existing)
- SBOM generation: `cargo-sbom`
- Dependency analysis: `cargo-tree`, `cargo-deny`

---

## 6. Vulnerability Disclosure History

| CVE | Severity | Date | Status | Details |
|-----|----------|------|--------|---------|
| N/A | N/A | - | Pre-1.0 | No disclosed vulnerabilities yet |

---

## 7. Compliance & Standards

### Targets

- ✅ CWE/OWASP compliance (no known high-risk patterns)
- 🔄 SLSA Framework: Targeting Level 2
- 📋 NIST Cybersecurity Framework

### Infrastructure

- ✅ GitHub Actions for CI/CD security
- ⚠️  Signing releases (TODO)
- ⚠️  Provenance attestation (TODO)

---

## 8. Security Configuration Guide

### For End-Users

**Minimal Security:**
```bash
export MEMOBUILD_CACHE_DIR=/var/cache/memobuild
chmod 700 /var/cache/memobuild
```

**Production Setup:**
```bash
# Use mutual TLS
export MEMOBUILD_TLS_CERT=/etc/memobuild/cert.pem
export MEMOBUILD_TLS_KEY=/etc/memobuild/key.pem

# Rate limiting
export MEMOBUILD_RATE_LIMIT_REQUESTS=1000
export MEMOBUILD_RATE_LIMIT_WINDOW_SECS=60

# Audit logging
export MEMOBUILD_JSON_LOGS=true
export RUST_LOG=memobuild=debug
```

---

## 9. Emergency Contacts

- **Security Team:** security@memobuild.dev
- **Incident Response:** oncall@memobuild.dev
- **Escalation:** maintainers@github.com/nrelab/MemoBuild

---

## 10. Future Roadmap

**v0.2.0 (Q2 2026):**
- [ ] Enforce CAS verification (DONE)
- [ ] Structured logging with audit trail
- [ ] Optional server authentication

**v1.0.0 (Q4 2026):**
- [ ] Mutual TLS support
- [ ] Keyring integration for token storage
- [ ] Signature verification for artifacts
- [ ] SLSA Level 3+

---

**For questions about security, contact: security@memobuild.dev**
