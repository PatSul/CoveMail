export type Provider =
  | "gmail"
  | "outlook"
  | "yahoo"
  | "icloud"
  | "fast_mail"
  | "proton_bridge"
  | "generic"
  | "exchange";

export interface OAuthProfile {
  client_id: string;
  auth_url: string;
  token_url: string;
  redirect_url: string;
  scopes: string[];
}

export interface Account {
  id: string;
  provider: Provider;
  protocols: string[];
  display_name: string;
  email_address: string;
  oauth_profile: OAuthProfile | null;
  created_at: string;
  updated_at: string;
}

export interface MailFolder {
  account_id: string;
  remote_id: string;
  path: string;
  delimiter: string | null;
  unread_count: number;
  total_count: number;
}

export interface MailThreadSummary {
  thread_id: string;
  subject: string;
  participants: string[];
  message_count: number;
  unread_count: number;
  most_recent_at: string;
}

export interface MailAddress {
  name?: string;
  address: string;
}

export interface MailFlags {
  seen: boolean;
  answered: boolean;
  flagged: boolean;
  deleted: boolean;
  draft: boolean;
  forwarded: boolean;
}

export interface MailAttachment {
  id: string;
  file_name: string;
  mime_type: string;
  size: number;
  inline: boolean;
}

export interface MailMessage {
  id: string;
  account_id: string;
  remote_id: string;
  thread_id: string;
  folder_path: string;
  from: MailAddress[];
  to: MailAddress[];
  cc: MailAddress[];
  bcc: MailAddress[];
  reply_to: MailAddress[];
  subject: string;
  preview: string;
  body_text?: string;
  body_html?: string;
  flags: MailFlags;
  labels: string[];
  headers?: Record<string, string>;
  attachments: MailAttachment[];
  sent_at?: string | null;
  received_at: string;
}

export interface SearchResult<T> {
  total: number;
  items: T[];
}

export interface ReminderTask {
  id: string;
  account_id: string;
  list_id: string;
  title: string;
  due_at: string | null;
  priority: "low" | "normal" | "high" | "critical";
  status: "not_started" | "in_progress" | "completed" | "canceled";
}

export interface AppConfig {
  version: number;
  profile_name: string;
  privacy: {
    telemetry_enabled: boolean;
    analytics_enabled: boolean;
    block_untrusted_remote_content: boolean;
    default_ai_mode: "local" | "cloud";
  };
  database: {
    file_name: string;
    sqlcipher_enabled: boolean;
  };
  sync: {
    email_poll_interval_secs: number;
    calendar_poll_interval_secs: number;
    task_poll_interval_secs: number;
    max_parallel_jobs: number;
  };
  ai: {
    local: {
      enabled: boolean;
      llama_cpp_binary: string | null;
      model_path: string | null;
      context_tokens: number;
      gpu_layers: number;
    };
    cloud: {
      enabled: boolean;
      per_feature_opt_in: boolean;
      feature_opt_in: string[];
      default_provider: string | null;
      providers: Record<string, { enabled: boolean; model: string; api_base: string | null }>;
    };
  };
  ui: {
    compact_density: boolean;
    default_start_page: string;
    timezone: string | null;
  };
}

export interface BootstrapResponse {
  config: AppConfig;
  accounts: Account[];
  pending_sync_jobs: number;
}

export interface SyncRunSummary {
  completed_jobs: number;
  failed_jobs: number;
  retried_jobs: number;
  email_messages_synced: number;
  calendar_events_synced: number;
  tasks_synced: number;
}

export interface DataProvenance {
  feature: string;
  mode: "local" | "cloud";
  destination: string;
  reason: string;
}

export interface BeginOAuthResponse {
  session_id: string;
  authorization_url: string;
}

export interface CompleteOAuthResponse {
  account: Account;
}

export interface OAuthBeginPayload {
  provider: Provider;
  email_address: string;
  display_name?: string | null;
  client_id: string;
  redirect_url: string;
}

export interface OAuthCompletePayload {
  session_id: string;
  csrf_state: string;
  code: string;
  calendar_id?: string | null;
  task_list_id?: string | null;
}

export interface OutgoingAttachment {
  file_name: string;
  mime_type: string;
  content_base64: string;
  inline: boolean;
}

export interface OutgoingMail {
  from: MailAddress;
  to: MailAddress[];
  cc: MailAddress[];
  bcc: MailAddress[];
  reply_to: MailAddress[];
  subject: string;
  body_text: string;
  body_html: string | null;
  attachments: OutgoingAttachment[];
}

export interface ValidateLocalAiRuntimePayload {
  llama_cpp_binary: string | null;
  model_path: string | null;
}

export interface ValidateLocalAiRuntimeResponse {
  valid: boolean;
  errors: string[];
}

export interface AiTaskExtractionResult {
  created: ReminderTask[];
  provenance: DataProvenance;
}
