CREATE TABLE IF NOT EXISTS accounts (
  id TEXT PRIMARY KEY,
  provider TEXT NOT NULL,
  protocols_json TEXT NOT NULL,
  display_name TEXT NOT NULL,
  email_address TEXT NOT NULL,
  oauth_profile_json TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS account_protocol_settings (
  account_id TEXT PRIMARY KEY,
  settings_json TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS mail_messages (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  remote_id TEXT NOT NULL,
  thread_id TEXT NOT NULL,
  folder_path TEXT NOT NULL,
  from_json TEXT NOT NULL,
  to_json TEXT NOT NULL,
  cc_json TEXT NOT NULL,
  bcc_json TEXT NOT NULL,
  reply_to_json TEXT NOT NULL,
  subject TEXT NOT NULL,
  preview TEXT NOT NULL,
  body_text TEXT,
  body_html TEXT,
  flags_json TEXT NOT NULL,
  labels_json TEXT NOT NULL,
  headers_json TEXT NOT NULL,
  attachments_json TEXT NOT NULL,
  sent_at TEXT,
  received_at TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(account_id, remote_id)
);

CREATE INDEX IF NOT EXISTS idx_mail_messages_account_folder ON mail_messages(account_id, folder_path);
CREATE INDEX IF NOT EXISTS idx_mail_messages_thread ON mail_messages(thread_id);
CREATE INDEX IF NOT EXISTS idx_mail_messages_received ON mail_messages(received_at DESC);

CREATE TABLE IF NOT EXISTS calendar_events (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  calendar_id TEXT NOT NULL,
  remote_id TEXT NOT NULL,
  title TEXT NOT NULL,
  description TEXT,
  location TEXT,
  timezone TEXT,
  starts_at TEXT NOT NULL,
  ends_at TEXT NOT NULL,
  all_day INTEGER NOT NULL,
  recurrence_rule TEXT,
  attendees_json TEXT NOT NULL,
  organizer TEXT,
  alarms_json TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(account_id, calendar_id, remote_id)
);

CREATE TABLE IF NOT EXISTS reminder_tasks (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  list_id TEXT NOT NULL,
  remote_id TEXT,
  title TEXT NOT NULL,
  notes TEXT,
  due_at TEXT,
  completed_at TEXT,
  priority TEXT NOT NULL,
  status TEXT NOT NULL,
  repeat_rule TEXT,
  parent_id TEXT,
  snoozed_until TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_reminder_tasks_due ON reminder_tasks(due_at);

CREATE TABLE IF NOT EXISTS sync_queue (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  domain TEXT NOT NULL,
  status TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  attempt_count INTEGER NOT NULL,
  max_attempts INTEGER NOT NULL,
  run_after TEXT NOT NULL,
  last_error TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sync_queue_next ON sync_queue(status, run_after);
