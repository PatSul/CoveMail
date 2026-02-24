# CoveMail Feature Implementation Plan

This document outlines a phased strategy to close the feature gaps across all competitor classes (Superhuman, Outlook/eM Client, Spark/Mailbird, and Thunderbird) while maintaining CoveMail's strict privacy and local-first architecture.

---

## Phase 1: Core Parity & Inbox Zero Foundation
**Goal:** Deliver the baseline expectations for a modern email client to compete with standard tools like Spark and Mailbird.

### 1. Unified Inbox & Account Aggregation
- **Architecture:** Expand the local Tantivy index and SQLite cache to seamlessly aggregate messages from multiple IMAP/OAuth sources.
- **UI/UX:** Create a "Smart Inbox" view that merges timelines, with clear visual indicators (badges or subtle tinting) for the source account.
- **Challenges:** Ensuring the `sync.max_parallel_jobs` and bounded concurrency handle simultaneous multi-account fetches without degrading local shell performance.

### 2. The "Inbox Zero" Triad: Snooze, Send Later, Undo Send
- **Snooze:** 
  - *Local Implementation:* Store snooze timestamps in SQLite; when the timer expires, move the message back to the inbox and trigger a local notification.
- **Send Later / Scheduled Send:**
  - *Local Implementation:* Keep the draft locally. A background worker picks it up and dispatches via SMTP at the scheduled time (requires the app to be running/backgrounded).
- **Undo Send:**
  - *Implementation:* Introduce an artificial local delay (e.g., 5-30 seconds) in the Outbox queue before dispatching to the SMTP server. Provide a toast notification with an "Undo" action to cancel the queue dispatch.

### 3. Basic Organization & Privacy Controls
- **1-Click Unsubscribe:** Detect `List-Unsubscribe` headers and provide a prominent UI button in the message viewer.
- **Read/Unread Optimization:** Ensure instantaneous local state updates for read indicators before the server sync completes.

---

## Phase 2: Power-User Productivity & Speed
**Goal:** Close the gap with Superhuman-class clients by focusing heavily on keyboard-centric workflows and triage speed.

### 1. Command Palette (Superhuman Parity)
- **Implementation:** Build a global `Cmd+K` / `Ctrl+K` interface in the native Rust shell (`egui`/`eframe`).
- **Functionality:** Map all triage actions (move, label, snooze, reply, search) to fuzzy-searchable commands. Ensure zero latency state transitions.

### 2. Split Inbox & Advanced Triage
- **Split Inbox:** Allow users to create horizontal or vertical splits based on saved queries (e.g., "Newsletters", "VIPs", "PRs").
- **Speed-Reader Workflows:** Implement Vim-like or standard sequential keyboard shortcuts (e.g., `j/k` for navigation, `e` for archive) to process the inbox without using a mouse.

### 3. Contact Management & Signatures (Outlook/eM Client Parity)
- **CRM Lite:** Create a local contacts module that aggregates communication history per sender, viewable in a sidebar next to the active thread.
- **Signatures & Templates:** Implement per-account rich text signatures and a snippet/template engine triggered by shortcodes (e.g., typing `/hello`).

---

## Phase 3: Advanced Organization & Collaboration
**Goal:** Satisfy Thunderbird and enterprise (Outlook) users requiring robust rules, taxonomy, and scheduling.

### 1. Local Rules Engine (Thunderbird Parity)
- **Implementation:** Build a local processing pipeline that evaluates incoming sync data against user-defined filters (if subject contains X, move to Y and mark as read).
- **Privacy Benefit:** Rules run entirely on the local machine against the SQLite cache, independent of provider capabilities (like Gmail filters).

### 2. Advanced Taxonomy & Search
- **Smart Folders / Saved Searches:** Expose the Tantivy query syntax through a UI builder to create persistent virtual folders (e.g., "Unread messages with attachments from @covemail.com over 5MB").
- **Custom Tagging:** Implement a local tagging system that overlays IMAP flags/labels, allowing arbitrary multi-tagging.

### 3. Calendar & Meeting Workflows (Outlook Parity)
- **Conversational Invites:** Parse ICS attachments and inject rich "Accept / Tentative / Decline" UI directly into the message viewer.
- **Workflow:** Implement RSVP tracking and comment flows (if supported by CalDAV/Graph).
- **Conflict Detection:** Query the local calendar cache when rendering an ICS invite and display warnings for overlapping events.

---

## Phase 4: The CoveMail Moat (Local AI & Data Portability)
**Goal:** Deliver the unique differentiators that no cloud-based competitor can match, solidifying the privacy-first mission.

### 1. Local AI Control Center
- **Thread Summaries & Drafts:** Utilize the integrated `llama.cpp` pipeline to read long threads and suggest replies. 
- **Security Constraint:** Enforce that these actions execute completely offline using local GGUF models.
- **Email-to-Task Pipeline:** Use local NLP (from `crates/cove-tasks`) to detect action items in emails and offer one-click creation of CalDAV/Google Tasks.

### 2. Data Portability & Transparency
- **Import/Export Tooling:** Build robust parsers for legacy `.mbox` and `.pst` files to ease migration from Thunderbird/Outlook.
- **Data Provenance Dashboards:** Add UI panels showing exactly what data is stored locally, with one-click purge options per account/domain.

---

## Technical Constraints & Execution Path

To deliver this plan, the engineering cadence must respect the following sequence:

1. **Storage & UI Foundation:** Tantivy and SQLx must be hardened first to support Split Inboxes, Rules, and Search.
2. **State Management:** The Rust native shell (`cove-native`) must implement robust background workers for Undo Send and Send Later queues.
3. **Keyboard Architecture:** The Command Palette requires a centralized action-dispatch system in the frontend/shell layer before individual macros can be wired.
4. **AI & Parsers:** Advanced ICS parsing, NLP, and rules execution sit on top of a stable IMAP/OAuth sync engine.
