# Contributing to Wayclick

## Development Setup

```sh
git clone https://github.com/nalcorso/wayclick.git
cd wayclick
./scripts/dev.sh      # Sets up permissions and example config
```

### With mise (recommended)

```sh
mise run build         # Build all crates
mise run check         # Run fmt + clippy + test + deny (full pre-commit check)
```

### Without mise

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

## Code Structure

See [ARCHITECTURE.md](ARCHITECTURE.md) for a full overview of the codebase.

## Making Changes

1. Fork the repository and create a feature branch.
2. Make your changes.
3. Run all checks: `mise run check` (or the manual commands above).
4. Submit a pull request.

## Testing

- **Unit tests** are co-located in each module file.
- **Integration tests** are in the `tests/` crate and exercise the IPC server
  end-to-end.
- **Uinput integration tests** require `/dev/uinput` write access and are gated
  behind the `integration` feature flag.

```sh
# All tests
cargo test --workspace

# Integration tests only
cargo test -p wayclick-tests

# Uinput tests (needs permissions)
cargo test -p wayclick-core --features integration
```

## Adding a New Action Type

1. Add the variant to `ActionConfig` in `config.rs`
2. Add the Lua API function in `lua_api.rs`
3. Add the execution logic in `engine.rs` (`execute_action_sync` and
   `execute_action_loop`)
4. Add unit tests for config parsing, Lua loading, and engine execution
5. Add an integration test that fires the trigger via IPC

## Code Style

- Follow standard Rust formatting (`cargo fmt`)
- Use `cargo clippy` to catch common issues
- Only add comments where the code needs clarification
- Keep functions focused and small
- Use `thiserror` for error types

## Commit Messages

Use conventional commits:

```
feat: add new scroll action type
fix: correct jitter calculation overflow
docs: update CONFIG_SCHEMA with scroll action
test: add integration test for scroll trigger
```
