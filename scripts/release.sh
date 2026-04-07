#!/usr/bin/env bash
# Release helper: bumps version, generates changelog, commits, and tags.
# The tag push triggers the GitHub Actions release workflow.
#
# Usage: ./scripts/release.sh <patch|minor|major>

set -euo pipefail

TYPE="${1:-}"
if [[ -z "$TYPE" ]] || [[ ! "$TYPE" =~ ^(patch|minor|major)$ ]]; then
    echo "Usage: $0 <patch|minor|major>"
    exit 1
fi

# Ensure clean working directory
if [[ -n "$(git status --porcelain)" ]]; then
    echo "Error: working directory is not clean. Commit or stash changes first."
    exit 1
fi

# Ensure on main branch
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$BRANCH" != "main" ]]; then
    echo "Error: must be on main branch (currently on ${BRANCH})"
    exit 1
fi

# Get current version
CURRENT_VERSION=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
echo "Current version: ${CURRENT_VERSION}"

# Bump version
IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT_VERSION"
case "$TYPE" in
    major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
    minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
    patch) PATCH=$((PATCH + 1)) ;;
esac
NEW_VERSION="${MAJOR}.${MINOR}.${PATCH}"
echo "New version: ${NEW_VERSION}"

# Update version in workspace Cargo.toml
sed -i '' "s/^version = \"${CURRENT_VERSION}\"/version = \"${NEW_VERSION}\"/" Cargo.toml
echo "Updated Cargo.toml"

# Update lockfile
cargo generate-lockfile 2>/dev/null
echo "Updated Cargo.lock"

# Generate changelog
if command -v git-cliff >/dev/null 2>&1; then
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

# Commit and tag
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore: release v${NEW_VERSION}"
git tag -a "v${NEW_VERSION}" -m "Release v${NEW_VERSION}"

echo ""
echo "Created tag v${NEW_VERSION}"
echo ""
echo "To trigger the release pipeline:"
echo "  git push && git push --tags"
