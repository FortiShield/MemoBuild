#!/usr/bin/env bash
# Security audit script for MemoBuild

set -uo pipefail

FAILURES=0

note_failure() {
    FAILURES=$((FAILURES + 1))
}

echo "🔐 MemoBuild Security Audit"
echo "============================"
echo ""

# 1. Cargo audit
echo "📋 Running cargo audit..."
if command -v cargo-audit >/dev/null 2>&1; then
    if cargo audit; then
        echo "✅ No known vulnerabilities found"
    else
        echo "⚠️  Some vulnerabilities found (see above)"
        note_failure
    fi
else
    echo "⚠️  cargo-audit not installed. Install with: cargo install cargo-audit"
    echo "   Skipping the vulnerability scan"
fi
echo ""

# 2. Dependency tree analysis
echo "📊 Checking dependency depth..."
dependency_tree="$(cargo tree --depth 3 2>/dev/null)"
tree_status=$?
if [ "$tree_status" -eq 0 ]; then
    echo "   $(printf '%s\n' "$dependency_tree" | wc -l) dependencies within three levels of the workspace"
else
    echo "⚠️  Could not analyze dependency tree"
    note_failure
fi
echo ""

# 3. Check for known insecure patterns
echo "🔍 Scanning for insecure patterns..."
patterns_found=0

# Check for unwrap in security-critical paths
if grep -r "\.unwrap()" src/export/registry.rs 2>/dev/null | grep -q "token"; then
    echo "⚠️  Found unwrap() on token handling"
    patterns_found=$((patterns_found + 1))
fi

# Check for debug assertions on secrets
if grep -r "eprintln!" src/server/mod.rs 2>/dev/null | grep -q "hash"; then
    echo "✅ Using logging for hashes (safe)"
else
    echo "⚠️  Check logging statements don't leak hashes"
fi

# Check for hardcoded credentials
if grep -r "password\|secret\|token" src/ 2>/dev/null | grep -q "= \""; then
    echo "⚠️  Possible hardcoded credential"
    patterns_found=$((patterns_found + 1))
fi

echo ""
echo "📊 Pattern scan complete. Issues found: $patterns_found"
echo ""

# 4. File permissions check (for deployment)
echo "🔒 Checking artifact storage directory..."
for cache_dir in .memobuild-cache "${HOME}/.memobuild/cache"; do
    if [ -d "$cache_dir" ]; then
        perms=$(stat -c %a "$cache_dir" 2>/dev/null || stat -f %OLp "$cache_dir" 2>/dev/null || echo "unknown")
        if [ "$perms" != "700" ]; then
            echo "⚠️  Cache directory ${cache_dir} permissions: $perms (recommended: 700)"
        else
            echo "✅ Cache directory ${cache_dir} has secure permissions"
        fi
    fi
done
echo ""

# 5. Test compilation with security features
echo "🧪 Testing with all security checks..."
check_output="$(cargo check --all-features 2>&1)"
check_status=$?
printf '%s\n' "$check_output" | grep -i "warn\|error" || true
if [ "$check_status" -ne 0 ]; then
    echo "❌ cargo check --all-features failed"
    note_failure
fi
echo ""

# 6. Run security-related tests
echo "✅ Running security tests..."
if cargo test --test error_handling_test -- --nocapture 2>&1; then
    echo "✅ Security tests passed"
else
    echo "❌ Security tests failed"
    note_failure
fi
echo ""

echo "================================"
if [ "$FAILURES" -eq 0 ]; then
    echo "✅ Security audit complete"
else
    echo "❌ Security audit finished with ${FAILURES} failing step(s)"
fi
echo ""
echo "Next steps:"
echo "- Review SECURITY.md for detailed recommendations"
echo "- Update dependencies: cargo update"
echo "- Use SBOM generation: cargo-sbom"
echo "- Enable JSON logging in production: MEMOBUILD_JSON_LOGS=true"

exit "$FAILURES"
