# Contributing

## Workflow

1. Fork and create a feature branch.
2. Run checks before opening a PR:
   - `cargo fmt --all`
   - `cargo check --workspace`
   - `cargo test --workspace`
   - `npm run build --prefix ui`
3. Keep changes scoped and include tests for new behavior.

## Commit style

Use clear, imperative commit messages.

## Security-sensitive areas

- Keychain handling
- OAuth flows
- AI provider integrations
- SQL migrations and data access

Treat these files as high-review surfaces.
