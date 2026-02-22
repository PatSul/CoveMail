-- Calendar: RSVP status for invite accept/decline
ALTER TABLE calendar_events ADD COLUMN rsvp_status TEXT DEFAULT 'needs_action';

-- Tasks: subtask ordering and priority sort
CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_id);
CREATE INDEX IF NOT EXISTS idx_tasks_priority ON tasks(priority);
CREATE INDEX IF NOT EXISTS idx_tasks_due ON tasks(due_at);
