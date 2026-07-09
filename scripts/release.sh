#!/usr/bin/env bash
# Release helper: bumps every version the release pipeline publishes,
# generates the changelog, commits, and tags. The tag push triggers the
# GitHub Actions release workflow, whose create-release job re-verifies
# that every manifest matches the tag.
#
# Usage: ./scripts/release.sh <patch|minor|major>

set -euo pipefail
cd "$(dirname "$0")/.."

TYPE="${1:-}"
if [[ -z "$TYPE" ]] || [[ ! "$TYPE" =~ ^(patch|minor|major)$ ]]; then
    echo "Usage: $0 <patch|minor|major>"
    exit 1
fi

# Ensure clean working directory
if [[ -n "$(git status --porcelain)" ]]; then
    echo "Error: working directory is not clean. Commit changes first."
    exit 1
fi

# Ensure on main branch
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$BRANCH" != "main" ]]; then
    echo "Error: must be on main branch (currently on ${BRANCH})"
    exit 1
fi

CURRENT_VERSION=$(grep -m1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
echo "Current version: ${CURRENT_VERSION}"

IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT_VERSION"
case "$TYPE" in
    major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
    minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
    patch) PATCH=$((PATCH + 1)) ;;
esac
NEW_VERSION="${MAJOR}.${MINOR}.${PATCH}"
echo "New version: ${NEW_VERSION}"

# Every version the pipeline publishes, in one pass (portable: BSD sed
# and GNU sed disagree on -i, so all edits go through python).
CURRENT_VERSION="$CURRENT_VERSION" NEW_VERSION="$NEW_VERSION" python3 - <<'EOF'
import json
import os
import re

current = os.environ["CURRENT_VERSION"]
new = os.environ["NEW_VERSION"]

# Workspace version (both crates inherit it).
path = "Cargo.toml"
src = open(path).read()
updated, n = re.subn(
    rf'^version = "{re.escape(current)}"$', f'version = "{new}"', src, count=1, flags=re.M
)
if n != 1:
    raise SystemExit(f"{path}: workspace version {current} not found")
open(path, "w").write(updated)
print(f"bumped {path}")

# mockpit-cli's registry pin on the mockpit dependency.
path = "crates/mockpit-cli/Cargo.toml"
src = open(path).read()
updated, n = re.subn(
    rf'mockpit = \{{ version = "{re.escape(current)}"', f'mockpit = {{ version = "{new}"', src, count=1
)
if n != 1:
    raise SystemExit(f"{path}: mockpit dep pin {current} not found")
open(path, "w").write(updated)
print(f"bumped {path}")

# npm packages, including platform-package pins in optionalDependencies.
for path in (
    "packages/core/package.json",
    "packages/mockpit/package.json",
    "packages/playwright/package.json",
    "crates/mockpit-napi/package.json",
    "crates/mockpit-cli/npm-shim/package.json",
):
    pkg = json.load(open(path))
    pkg["version"] = new
    for name, pinned in pkg.get("optionalDependencies", {}).items():
        if pinned == current:
            pkg["optionalDependencies"][name] = new
    with open(path, "w") as f:
        json.dump(pkg, f, indent=2)
        f.write("\n")
    print(f"bumped {path}")
EOF

# Refresh lockfiles for the new workspace versions. bun resolves against
# the public registry regardless of any local corporate npmrc profile.
cargo update --workspace --quiet
echo "Updated Cargo.lock"
NPM_CONFIG_REGISTRY=https://registry.npmjs.org bun install --silent
echo "Updated bun.lock"

# Generate changelog
if command -v git-cliff >/dev/null 2>&1; then
    [[ -f CHANGELOG.md ]] || touch CHANGELOG.md
    PREV_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
    if [[ -n "$PREV_TAG" ]]; then
        git-cliff "${PREV_TAG}..HEAD" --tag "v${NEW_VERSION}" --prepend CHANGELOG.md
    else
        git-cliff --tag "v${NEW_VERSION}" --output CHANGELOG.md
    fi
    echo "Generated changelog"
else
    echo "Warning: git-cliff not found, skipping changelog generation"
fi

git add Cargo.toml Cargo.lock bun.lock CHANGELOG.md \
    crates/mockpit-cli/Cargo.toml \
    crates/mockpit-napi/package.json \
    crates/mockpit-cli/npm-shim/package.json \
    packages/core/package.json \
    packages/mockpit/package.json \
    packages/playwright/package.json
git commit -m "chore: release v${NEW_VERSION}"
git tag -a "v${NEW_VERSION}" -m "Release v${NEW_VERSION}"

echo ""
echo "Created tag v${NEW_VERSION}"
echo ""
echo "To trigger the release pipeline:"
echo "  git push && git push --tags"
