# Cove Mail

Cove Mail is a privacy-first, AI-assisted email, calendar, and reminders desktop client built with Rust and Tauri v2.

## Privacy Contract

- No telemetry
- No analytics
- No external calls except user-configured account providers and explicitly opted-in AI providers
- Credentials and API keys are stored in the OS keychain
- Local-first data model: SQLite cache + local full-text index
- Local AI is the default mode

## Tech Stack

- Rust stable + Tokio
- Tauri v2 desktop shell (Windows, macOS, Linux)
- React + Vite frontend
- SQLx + SQLite local cache
- Tantivy full-text search
- TOML user config
- Keychain secrets with `keyring`

## Repository Layout

- `src-tauri`: Desktop app entrypoint and Tauri command surface
- `crates/cove-native`: Native Rust (`egui/eframe`) desktop shell
- `crates/cove-core`: Shared domain models
- `crates/cove-config`: TOML config manager and defaults
- `crates/cove-security`: Keychain and OAuth PKCE workflows
- `crates/cove-storage`: SQLx repositories, migrations, Tantivy integration
- `crates/cove-email`: Email services and protocol adapters
- `crates/cove-calendar`: Calendar services + ICS import/export
- `crates/cove-tasks`: Reminders/tasks and natural language parsing
- `crates/cove-ai`: Local (llama.cpp) and optional cloud AI providers
- `ui`: React frontend

## Current Protocol Adapter Coverage

- Email: IMAP/SMTP adapter boundary, JMAP and EWS adapters scaffolded, SMTP sending implemented
- Calendar: CalDAV/Google/Graph adapter boundaries, ICS import/export implemented
- Tasks: CalDAV VTODO/Google Tasks/Graph To Do adapter boundaries + local natural-language capture

## Local Development

### Prerequisites

- Rust stable toolchain
- Node.js 20+
- `npm`

### Install and run

```bash
npm install --prefix ui
cargo check --workspace
npm run dev --prefix ui
cargo tauri dev --manifest-path src-tauri/Cargo.toml

# Native Rust desktop shell (no web frontend)
cargo run -p cove-native
```

## Security Notes

- SQLCipher support requires linking against a SQLCipher-enabled SQLite build and setting a key in keychain namespace `database` id `sqlcipher_key`.
- Cloud AI keys are stored in keychain namespace `ai_api_key`.

## Open Source

Dual-licensed under AGPL-3.0 (for application components) and MIT (for libraries). See `LICENSE`.
