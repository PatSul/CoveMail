CREATE TABLE IF NOT EXISTS mail_attachment_content (
  attachment_id TEXT PRIMARY KEY,
  message_id TEXT NOT NULL,
  account_id TEXT NOT NULL,
  content BLOB NOT NULL,
  FOREIGN KEY(message_id) REFERENCES mail_messages(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_attachment_content_message
  ON mail_attachment_content(message_id);
