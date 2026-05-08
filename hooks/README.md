# Git Hooks

Custom Git hooks for the All4One project.

## Setup

The hooks are automatically enabled when you clone or fetch from the repository. Git will use these hooks from the `core.hooksPath` configuration.

If you're working on an existing clone and haven't run `git config core.hooksPath hooks` yet:

```bash
git config core.hooksPath hooks
```

## Hooks

### pre-push

Runs before each push to ensure code quality:

1. **Format**: `cargo fmt --all` - Automatically formats all Rust code
2. **Verify**: `cargo fmt --all -- --check` - Ensures formatting compliance
3. **Lint**: `cargo clippy --workspace --all-targets -- -D warnings` - Strict linting

All checks must pass before push is allowed.

### Bypassing hooks (use with caution)

To push without running hooks:

```bash
git push --no-verify
```

**Note:** This is not recommended. Ensure your code passes all checks before bypassing.

## Adding new hooks

1. Create a new executable file in this directory (e.g., `pre-commit`)
2. Make it executable: `git update-index --chmod=+x hooks/pre-commit`
3. Commit the change

Git will automatically run it at the appropriate stage.
