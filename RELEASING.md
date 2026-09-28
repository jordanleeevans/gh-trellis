# Releasing Trellis

This document describes the process for releasing new versions of Trellis and maintaining the changelog.

## Semantic Versioning Policy

Trellis follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as of v0.1.0:

- **Pre-1.0.0 versions (0.y.z)**: 
  - `y` (minor version): Increment for new features or **breaking changes** (since we're pre-1.0, breaking changes are expected as the API stabilizes)
  - `z` (patch version): Increment for bug fixes and non-breaking improvements
- **Post-1.0.0 versions (x.y.z)**:
  - `x` (major version): Increment for breaking changes only
  - `y` (minor version): Increment for new features (backwards compatible)
  - `z` (patch version): Increment for bug fixes

## Commit Format

Trellis uses [Conventional Commits](https://www.conventionalcommits.org/) to enable automated changelog generation. Please use these prefixes:

- `feat:` - A new feature (appears in CHANGELOG under "Features")
- `fix:` - A bug fix (appears in CHANGELOG under "Bug Fixes")
- `docs:` - Documentation changes (appears in CHANGELOG under "Documentation")
- `perf:` - Performance improvements (appears in CHANGELOG under "Performance")
- `refactor:` - Code refactoring (appears in CHANGELOG under "Refactoring")
- `style:` - Code style changes (appears in CHANGELOG under "Styling")
- `test:` - Test changes (appears in CHANGELOG under "Testing")
- `chore:` - Build configuration, dependencies, etc. (appears in CHANGELOG under "Miscellaneous Tasks")

To mark a commit as breaking, include `BREAKING CHANGE:` in the commit message footer:

```
feat: redesign API response format

BREAKING CHANGE: the response structure has changed
```

## Changelog Generation

The changelog is automatically generated from git history using [git-cliff](https://github.com/orhun/git-cliff) and the configuration in [`cliff.toml`](./cliff.toml).

### Regenerating the Changelog

To regenerate the entire `CHANGELOG.md` from git history:

```bash
git-cliff -o CHANGELOG.md
```

To generate a changelog for a specific tag:

```bash
git-cliff --tag v0.2.0 -o CHANGELOG.md
```

Note: By default, git-cliff will generate a single unreleased section at the top. Tagged releases are automatically categorized with their dates.

## Release Steps

Follow these steps to make a release:

1. **Verify all changes are committed** and the working tree is clean:
   ```bash
   git status
   ```

2. **Determine the new version** based on the commits since the last release and the semver policy above. For pre-1.0 releases, breaking changes increment the minor version.

3. **Update the version in `Cargo.toml`**:
   ```bash
   # Edit Cargo.toml and change the version
   # Example: version = "0.2.0"
   ```

4. **Regenerate the changelog**:
   ```bash
   git-cliff --tag v0.2.0 -o CHANGELOG.md
   ```
   
   This will move unreleased changes into a dated release section.

5. **Commit the changes**:
   ```bash
   git add Cargo.toml CHANGELOG.md
   git commit -m "chore(release): bump version to v0.2.0"
   ```

6. **Create a git tag** for the release:
   ```bash
   git tag v0.2.0
   ```

7. **Push the commit and tag** to the repository:
   ```bash
   git push origin main
   git push origin v0.2.0
   ```

## Tools

- **git-cliff**: Generates changelogs from conventional commits
  - Install: `cargo install git-cliff` or `brew install git-cliff`
  - Docs: https://github.com/orhun/git-cliff
- **Conventional Commits**: Specification for commit messages
  - Docs: https://www.conventionalcommits.org/

## Notes

- The changelog header and formatting are defined in `cliff.toml`
- Merge commits and squash-merge commits are automatically filtered out
- The first tagged release will be `v0.1.0` when ready
