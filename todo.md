# Cove Mail TODO

Last updated: 2026-02-19

## Mission
- Build a production-ready, privacy-first desktop client for email, calendar, and reminders.
- Keep local-first behavior as default.
- Move to an all-native Rust primary experience (no web frontend dependency for core UX).

## Priority Legend
- P0 = release blocker
- P1 = must-have for competitive parity
- P2 = strong differentiator

## P0 - Release Blockers
- [ ] Make native Rust shell the primary desktop UX and reach feature parity with current Tauri app
  - [x] Account picker and onboarding flow
  - [x] Folder tree + thread list + message viewer
  - [x] Compose + attachments + send
  - [x] Calendar and task views in the native shell
- [ ] Wire OAuth2 PKCE onboarding end-to-end in production UI
  - [x] Gmail
  - [x] Microsoft 365 / Outlook
  - [x] Generic provider onboarding guardrails
- [x] Finish sync engine hardening
  - [x] Respect `sync.max_parallel_jobs`
  - [x] Add bounded global queue concurrency (`run_sync_queue`)
  - [x] Keep retry/backoff + dead-letter semantics
  - [x] Add bounded concurrency per domain/account
  - [x] Add sync health metrics in-app (local only)
- [x] Complete inbox core UX (not search-only)
  - [x] Folder/label browser
  - [x] Thread-first list with unread/flag state
  - [x] Full message rendering (safe HTML policy)
  - [x] Attachment list/open/save with strict checks
- [x] Notifications
  - [x] Sync summary notifications (desktop + in-app)
  - [x] New message notifications (per account/folder policy)
  - [x] Reminder due/snooze notifications
  - [x] Sync failure notifications with actionable recovery
- [x] Security hardening pass
  - [x] Restrict Tauri capability scope (`shell:allow-open`) or remove if native path becomes primary
  - [x] Keep strict secret namespace/id validation
  - [x] Enforce SQLCipher toggle + key existence constraints
  - [x] Confirm no secret logging anywhere
  - [x] Threat model + abuse-case test checklist
- [x] macOS window polish
  - [x] Validate traffic-light spacing against overlay titlebar at multiple scale factors
  - [x] Add safe top inset in UI layout where needed
  - [x] Verify no overlap with window drag/header controls

## P1 - Competitive Parity (Mailbird/Outlook/Thunderbird/Spark class)
- [x] Unified inbox across accounts
- [x] Priority inbox
- [x] Message read indicator
- [x] Send later
- [x] Email snooze, pin, auto archive & more
- [x] Undo send window
- [x] Signatures (global and per account) + custom email templates/snippets
- [x] Mentions
- [x] 1-click unsubscribe
- [x] Rules/filters (local + provider-backed where available)
- [x] Follow-up reminders / reply tracking workflows
- [x] Keyboard shortcut map and command palette
- [x] Better search UX (saved searches, filter chips, date/people facets)
- [x] Calendar parity
  - [x] Unified Calendar across accounts
  - [x] Recurrence editor + exceptions
  - [x] Invite accept/decline workflow
  - [x] Conflict detection surfaced in UI
  - [x] Timezone controls and calendar overlays
- [x] Notes and Tasks parity
  - [x] Dedicated Notes workspace
  - [x] Subtasks UI
  - [x] Repeat rule editor
  - [x] Priority and grouping views

## P2 - Differentiators
- [x] Local AI control center
  - [x] llama.cpp path validation UI
  - [x] GGUF model validation + quick diagnostics
  - [x] Per-feature cloud AI opt-in toggles with clear provenance labels
- [x] AI workflows
  - [x] High quality thread summaries
  - [x] Draft reply suggestions
  - [x] Translate messages (using AI)
  - [x] Email -> action item -> task pipeline confirmation UX
  - [x] Scheduling assistant suggestions from email context
- [x] Advanced privacy UX
  - [x] Data provenance panel for each feature/action
  - [x] One-click data export/purge by account/domain
  - [x] Remote content/tracker blocking transparency

## Protocol and Provider Completion
- [x] Email providers
  - [x] Gmail (OAuth2 + IMAP/SMTP) full tested path
  - [x] Outlook/Microsoft 365 (OAuth2 + IMAP/SMTP and Graph where applicable)
  - [x] Yahoo, iCloud, FastMail
  - [x] Proton Mail Bridge local IMAP/SMTP path
  - [x] Generic IMAP/SMTP onboarding wizard + validation
- [x] Calendar providers
  - [x] CalDAV
  - [x] Google Calendar API
  - [x] Microsoft Graph Calendar API
  - [x] ICS import/export polish
- [x] Tasks providers
  - [x] CalDAV VTODO
  - [x] Google Tasks API
  - [x] Microsoft To Do API
- [x] Cloud Storage Integrations (Attachments & Links)
  - [x] Google Drive, Dropbox, OneDrive, etc.

## Security and Compliance Checklist
- [x] TLS everywhere via rustls (no OpenSSL dependency path)
- [x] Secrets only in OS keychain (`account_password`, `oauth_access_token`, `oauth_refresh_token`, `ai_api_key`, `database/sqlcipher_key`)
- [x] CSP/permission review for every release
- [x] Dependency audit + supply-chain scanning in CI
- [x] Fuzz tests on parser boundaries (ICS, MIME, EWS/JMAP response parsing)
- [x] Secure defaults documented in `README.md` and `SECURITY.md`

## UX and Theme System
- [x] Theme system complete and consistent across all views
  - [x] Light and dark quality pass
  - [x] High-contrast quality pass
  - [x] Typography scale and spacing consistency audit
  - [x] Cross-platform widget behavior audit (macOS/Windows/Linux)
- [x] "Kick-ass" polish pass
  - [x] Better empty states and onboarding visuals
  - [x] Motion timing/intent consistency
  - [x] High-density layout mode for power users

## Competitive Gap Matrix
- [x] Mailbird parity gaps
  - [x] Unified inbox rules customization UI
  - [x] Speed-reader + quick-reply keyboard workflows
  - [x] App integrations panel equivalent (Slack/WhatsApp/Asana/etc.)
- [x] Superhuman-class productivity gaps
  - [x] Command palette with full triage actions
  - [x] Split inbox and advanced follow-up automation
  - [x] Read status/response-time workflow dashboards
- [x] eM Client / Outlook-class collaboration gaps
  - [x] Contact management + signatures with per-account templates
  - [x] Meeting response UX (accept/tentative/decline) with comment flows
  - [x] Shared mailbox / delegated account handling
- [x] Thunderbird-class power-user gaps
  - [x] Rules engine with robust local filtering and actions
  - [x] Advanced tag taxonomy and saved search folders
  - [x] Import tooling from legacy mailbox formats

## Quality Gates Before 1.0
- [x] End-to-end tests for account onboarding + sync + send
- [x] Offline mode resilience tests (network cut/reconnect)
- [x] Soak tests for long-running sync/IDLE listeners
- [x] Performance budget and startup-time targets
- [x] Crash reporting strategy (local logs only, no telemetry by default)

## Immediate Execution Order
1. [x] Native Rust inbox/calendar/tasks shell parity (P0)
2. [x] OAuth onboarding UX + provider defaults hardened (P0)
3. [x] Sync engine concurrency + reliability upgrades (P0)
4. [x] Notifications + reminder alarm UX (P0)
5. [x] Security surface lockdown + threat-model pass (P0)
6. [x] Competitive parity features (P1)
7. [x] AI differentiators and advanced UX (P2)
