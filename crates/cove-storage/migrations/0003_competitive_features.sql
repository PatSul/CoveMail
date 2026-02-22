-- Snooze, pin, send-later columns on mail_messages
ALTER TABLE mail_messages ADD COLUMN snoozed_until TEXT;
ALTER TABLE mail_messages ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
ALTER TABLE mail_messages ADD COLUMN send_at TEXT;

CREATE INDEX IF NOT EXISTS idx_mail_messages_snoozed
  ON mail_messages(snoozed_until) WHERE snoozed_until IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_mail_messages_pinned
  ON mail_messages(pinned) WHERE pinned = 1;
CREATE INDEX IF NOT EXISTS idx_mail_messages_send_at
  ON mail_messages(send_at) WHERE send_at IS NOT NULL;

-- Signatures
CREATE TABLE IF NOT EXISTS email_signatures (
  id TEXT PRIMARY KEY,
  account_id TEXT,
  name TEXT NOT NULL,
  body_html TEXT NOT NULL,
  body_text TEXT NOT NULL,
  is_default INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

-- Templates
CREATE TABLE IF NOT EXISTS email_templates (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  subject TEXT NOT NULL,
  body_html TEXT NOT NULL,
  body_text TEXT NOT NULL
);

-- Rules / Filters
CREATE TABLE IF NOT EXISTS mail_rules (
  id TEXT PRIMARY KEY,
  account_id TEXT,
  name TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  conditions_json TEXT NOT NULL,
  match_all INTEGER NOT NULL DEFAULT 1,
  actions_json TEXT NOT NULL,
  stop_processing INTEGER NOT NULL DEFAULT 0,
  sort_order INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

-- Contacts
CREATE TABLE IF NOT EXISTS contacts (
  id TEXT PRIMARY KEY,
  account_id TEXT,
  email TEXT NOT NULL,
  display_name TEXT,
  phone TEXT,
  organization TEXT,
  notes TEXT,
  last_contacted TEXT,
  contact_count INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE SET NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_contacts_email
  ON contacts(email);
CREATE INDEX IF NOT EXISTS idx_contacts_name
  ON contacts(display_name);
CREATE INDEX IF NOT EXISTS idx_contacts_count
  ON contacts(contact_count DESC);
