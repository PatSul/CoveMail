# AegisInbox TODO

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
  - [ ] Generic provider onboarding guardrails
- [ ] Finish sync engine hardening
  - [x] Respect `sync.max_parallel_jobs`
  - [x] Add bounded global queue concurrency (`run_sync_queue`)
  - [x] Keep retry/backoff + dead-letter semantics
  - [ ] Add bounded concurrency per domain/account
  - [x] Add sync health metrics in-app (local only)
- [ ] Complete inbox core UX (not search-only)
  - [x] Folder/label browser
  - [x] Thread-first list with unread/flag state
  - [ ] Full message rendering (safe HTML policy)
  - [ ] Attachment list/open/save with strict checks
- [ ] Notifications
  - [x] Sync summary notifications (desktop + in-app)
  - [ ] New message notifications (per account/folder policy)
  - [ ] Reminder due/snooze notifications
  - [x] Sync failure notifications with actionable recovery
- [ ] Security hardening pass
  - [x] Restrict Tauri capability scope (`shell:allow-open`) or remove if native path becomes primary
  - [x] Keep strict secret namespace/id validation
  - [x] Enforce SQLCipher toggle + key existence constraints
  - [ ] Confirm no secret logging anywhere
  - [ ] Threat model + abuse-case test checklist
- [ ] macOS window polish
  - [ ] Validate traffic-light spacing against overlay titlebar at multiple scale factors
  - [x] Add safe top inset in UI layout where needed
  - [ ] Verify no overlap with window drag/header controls

## P1 - Competitive Parity (Mailbird/Outlook/Thunderbird/Spark class)
- [ ] Unified inbox across accounts
- [ ] Priority inbox
- [ ] Message read indicator
- [ ] Send later
- [ ] Email snooze, pin, auto archive & more
- [ ] Undo send window
- [ ] Signatures (global and per account) + custom email templates/snippets
- [ ] Mentions
- [ ] 1-click unsubscribe
- [ ] Rules/filters (local + provider-backed where available)
- [ ] Follow-up reminders / reply tracking workflows
- [ ] Keyboard shortcut map and command palette
- [ ] Better search UX (saved searches, filter chips, date/people facets)
- [ ] Calendar parity
  - [ ] Unified Calendar across accounts
  - [ ] Recurrence editor + exceptions
  - [ ] Invite accept/decline workflow
  - [ ] Conflict detection surfaced in UI
  - [ ] Timezone controls and calendar overlays
- [ ] Notes and Tasks parity
  - [ ] Dedicated Notes workspace
  - [ ] Subtasks UI
  - [ ] Repeat rule editor
  - [ ] Priority and grouping views

## P2 - Differentiators
- [ ] Local AI control center
  - [x] llama.cpp path validation UI
  - [x] GGUF model validation + quick diagnostics
  - [ ] Per-feature cloud AI opt-in toggles with clear provenance labels
- [ ] AI workflows
  - [ ] High quality thread summaries
  - [ ] Draft reply suggestions
  - [ ] Translate messages (using AI)
  - [x] Email -> action item -> task pipeline confirmation UX
  - [ ] Scheduling assistant suggestions from email context
- [ ] Advanced privacy UX
  - [ ] Data provenance panel for each feature/action
  - [ ] One-click data export/purge by account/domain
  - [ ] Remote content/tracker blocking transparency

## Protocol and Provider Completion
- [ ] Email providers
  - [ ] Gmail (OAuth2 + IMAP/SMTP) full tested path
  - [ ] Outlook/Microsoft 365 (OAuth2 + IMAP/SMTP and Graph where applicable)
  - [ ] Yahoo, iCloud, FastMail
  - [ ] Proton Mail Bridge local IMAP/SMTP path
  - [ ] Generic IMAP/SMTP onboarding wizard + validation
- [ ] Calendar providers
  - [ ] CalDAV
  - [ ] Google Calendar API
  - [ ] Microsoft Graph Calendar API
  - [ ] ICS import/export polish
- [ ] Tasks providers
  - [ ] CalDAV VTODO
  - [ ] Google Tasks API
  - [ ] Microsoft To Do API
- [ ] Cloud Storage Integrations (Attachments & Links)
  - [ ] Google Drive, Dropbox, OneDrive, etc.

## Security and Compliance Checklist
- [ ] TLS everywhere via rustls (no OpenSSL dependency path)
- [ ] Secrets only in OS keychain (`account_password`, `oauth_access_token`, `oauth_refresh_token`, `ai_api_key`, `database/sqlcipher_key`)
- [ ] CSP/permission review for every release
- [ ] Dependency audit + supply-chain scanning in CI
- [ ] Fuzz tests on parser boundaries (ICS, MIME, EWS/JMAP response parsing)
- [ ] Secure defaults documented in `README.md` and `SECURITY.md`

## UX and Theme System
- [ ] Theme system complete and consistent across all views
  - [x] Light and dark quality pass
  - [ ] High-contrast quality pass
  - [x] Typography scale and spacing consistency audit
  - [ ] Cross-platform widget behavior audit (macOS/Windows/Linux)
- [ ] "Kick-ass" polish pass
  - [x] Better empty states and onboarding visuals
  - [x] Motion timing/intent consistency
  - [ ] High-density layout mode for power users

## Competitive Gap Matrix
- [ ] Mailbird parity gaps
  - [ ] Unified inbox rules customization UI
  - [ ] Speed-reader + quick-reply keyboard workflows
  - [ ] App integrations panel equivalent (Slack/WhatsApp/Asana/etc.)
- [ ] Superhuman-class productivity gaps
  - [ ] Command palette with full triage actions
  - [ ] Split inbox and advanced follow-up automation
  - [ ] Read status/response-time workflow dashboards
- [ ] eM Client / Outlook-class collaboration gaps
  - [ ] Contact management + signatures with per-account templates
  - [ ] Meeting response UX (accept/tentative/decline) with comment flows
  - [ ] Shared mailbox / delegated account handling
- [ ] Thunderbird-class power-user gaps
  - [ ] Rules engine with robust local filtering and actions
  - [ ] Advanced tag taxonomy and saved search folders
  - [ ] Import tooling from legacy mailbox formats

## Quality Gates Before 1.0
- [ ] End-to-end tests for account onboarding + sync + send
- [ ] Offline mode resilience tests (network cut/reconnect)
- [ ] Soak tests for long-running sync/IDLE listeners
- [ ] Performance budget and startup-time targets
- [ ] Crash reporting strategy (local logs only, no telemetry by default)

## Immediate Execution Order
1. [ ] Native Rust inbox/calendar/tasks shell parity (P0)
2. [ ] OAuth onboarding UX + provider defaults hardened (P0)
3. [ ] Sync engine concurrency + reliability upgrades (P0)
4. [ ] Notifications + reminder alarm UX (P0)
5. [ ] Security surface lockdown + threat-model pass (P0)
6. [ ] Competitive parity features (P1)
7. [ ] AI differentiators and advanced UX (P2)
