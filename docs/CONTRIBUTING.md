# Contributing to Wayclick

## Development Setup

```sh
git clone <repo>
cd wayclick
./scripts/dev.sh      # Sets up permissions and example config
cargo build --workspace
cargo test --workspace
```

## Code Structure

See [ARCHITECTURE.md](ARCHITECTURE.md) for a full overview of the codebase.

## Making Changes

1. Fork the repository and create a feature branch.
2. Make your changes.
3. Ensure all tests pass: `cargo test --workspace`
4. Ensure no warnings: `cargo build --workspace`
5. Run clippy: `cargo clippy --workspace`
6. Submit a pull request.

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
