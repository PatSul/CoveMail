import type {
  Account,
  AiTaskExtractionResult,
  AppConfig,
  BeginOAuthResponse,
  BootstrapResponse,
  CompleteOAuthResponse,
  DataProvenance,
  MailFolder,
  MailMessage,
  MailThreadSummary,
  OAuthBeginPayload,
  OAuthCompletePayload,
  OutgoingMail,
  ReminderTask,
  SearchResult,
  SyncRunSummary,
  ValidateLocalAiRuntimePayload,
  ValidateLocalAiRuntimeResponse,
} from "../types";

async function getInvoke() {
  try {
    const module = await import("@tauri-apps/api/core");
    return module.invoke;
  } catch {
    return null;
  }
}

export async function bootstrap(): Promise<BootstrapResponse> {
  const invoke = await getInvoke();
  if (!invoke) {
    return {
      config: {
        version: 1,
        profile_name: "browser-dev",
        privacy: {
          telemetry_enabled: false,
          analytics_enabled: false,
          block_untrusted_remote_content: true,
          default_ai_mode: "local",
        },
        database: {
          file_name: "covemail.sqlite3",
          sqlcipher_enabled: false,
        },
        sync: {
          email_poll_interval_secs: 120,
          calendar_poll_interval_secs: 300,
          task_poll_interval_secs: 300,
          max_parallel_jobs: 4,
        },
        ai: {
          local: {
            enabled: true,
            llama_cpp_binary: null,
            model_path: null,
            context_tokens: 4096,
            gpu_layers: 32,
          },
          cloud: {
            enabled: false,
            per_feature_opt_in: true,
            feature_opt_in: [],
            default_provider: null,
            providers: {},
          },
        },
        ui: {
          compact_density: false,
          default_start_page: "inbox",
          timezone: null,
        },
      },
      accounts: [],
      pending_sync_jobs: 0,
    };
  }

  return invoke<BootstrapResponse>("bootstrap");
}

export async function saveConfig(config: AppConfig): Promise<void> {
  const invoke = await getInvoke();
  if (!invoke) return;
  await invoke("save_config", { config });
}

export async function listAccounts(): Promise<Account[]> {
  const invoke = await getInvoke();
  return invoke ? invoke<Account[]>("list_accounts") : [];
}

export async function queueEmailSync(accountId: string): Promise<void> {
  const invoke = await getInvoke();
  if (!invoke) return;

  await invoke("queue_sync_job", {
    payload: {
      account_id: accountId,
      domain: "email",
      payload: {},
      run_after_secs: 0,
    },
  });
}

export async function runSyncQueue(): Promise<SyncRunSummary> {
  const invoke = await getInvoke();
  return invoke
    ? invoke<SyncRunSummary>("run_sync_queue")
    : {
        completed_jobs: 0,
        failed_jobs: 0,
        retried_jobs: 0,
        email_messages_synced: 0,
        calendar_events_synced: 0,
        tasks_synced: 0,
      };
}

export async function listMailFolders(
  accountId: string,
  refreshRemote = false
): Promise<MailFolder[]> {
  const invoke = await getInvoke();
  if (!invoke) return [];

  return invoke("list_mail_folders", {
    payload: {
      account_id: accountId,
      refresh_remote: refreshRemote,
    },
  });
}

export async function listMailThreads(
  accountId: string,
  folder?: string,
  limit = 120,
  offset = 0
): Promise<MailThreadSummary[]> {
  const invoke = await getInvoke();
  if (!invoke) return [];

  return invoke("list_mail_threads", {
    payload: {
      account_id: accountId,
      folder: folder ?? null,
      limit,
      offset,
    },
  });
}

export async function listThreadMessages(accountId: string, threadId: string): Promise<MailMessage[]> {
  const invoke = await getInvoke();
  if (!invoke) return [];

  return invoke("list_thread_messages", {
    payload: {
      account_id: accountId,
      thread_id: threadId,
    },
  });
}

export async function sendMail(accountId: string, outgoing: OutgoingMail): Promise<void> {
  const invoke = await getInvoke();
  if (!invoke) {
    throw new Error("Sending mail requires the Tauri runtime");
  }

  await invoke("send_mail", {
    payload: {
      account_id: accountId,
      outgoing,
    },
  });
}

export async function searchMail(query: string): Promise<SearchResult<MailMessage>> {
  const invoke = await getInvoke();
  if (!invoke) return { total: 0, items: [] };

  return invoke("search_mail", {
    payload: {
      query,
      limit: 40,
    },
  });
}

export async function listTasks(accountId: string): Promise<ReminderTask[]> {
  const invoke = await getInvoke();
  if (!invoke) return [];
  return invoke("list_tasks", { accountId });
}

export async function createTaskFromText(accountId: string, text: string): Promise<ReminderTask> {
  const invoke = await getInvoke();
  if (!invoke) {
    throw new Error("Task creation requires the Tauri runtime");
  }

  return invoke("create_task_from_text", {
    payload: {
      account_id: accountId,
      list_id: "default",
      text,
    },
  });
}

export async function importIcs(accountId: string, icsPayload: string): Promise<number> {
  const invoke = await getInvoke();
  if (!invoke) return 0;

  const events = await invoke<Array<unknown>>("import_calendar_ics", {
    payload: {
      account_id: accountId,
      calendar_id: "default",
      ics_payload: icsPayload,
    },
  });

  return events.length;
}

export async function exportIcs(accountId: string): Promise<string> {
  const invoke = await getInvoke();
  if (!invoke) return "";

  const now = new Date();
  const from = new Date(now.getTime() - 1000 * 60 * 60 * 24 * 30);
  const to = new Date(now.getTime() + 1000 * 60 * 60 * 24 * 365);

  return invoke<string>("export_calendar_ics", {
    payload: {
      account_id: accountId,
      from: from.toISOString(),
      to: to.toISOString(),
    },
  });
}

export async function aiSummarize(
  subject: string,
  body: string,
  mode: "local" | "cloud"
): Promise<{ output: string; provenance: DataProvenance }> {
  const invoke = await getInvoke();
  if (!invoke) {
    return {
      output: "Local runtime unavailable in browser preview.",
      provenance: {
        feature: "email_summarization",
        mode: "local",
        destination: "browser_dev",
        reason: "Tauri runtime not attached",
      },
    };
  }

  return invoke("ai_summarize_email", {
    payload: {
      subject,
      body,
      mode,
      cloud_provider: mode === "cloud" ? "open_ai" : null,
    },
  });
}

export async function beginOAuthPkce(payload: OAuthBeginPayload): Promise<BeginOAuthResponse> {
  const invoke = await getInvoke();
  if (!invoke) {
    throw new Error("OAuth onboarding requires the Tauri runtime");
  }

  return invoke("begin_oauth_pkce", { payload });
}

export async function completeOAuthPkce(
  payload: OAuthCompletePayload
): Promise<CompleteOAuthResponse> {
  const invoke = await getInvoke();
  if (!invoke) {
    throw new Error("OAuth onboarding requires the Tauri runtime");
  }

  return invoke("complete_oauth_pkce", { payload });
}

export async function aiSuggestReply(
  subject: string,
  body: string,
  mode: "local" | "cloud"
): Promise<{ output: string; provenance: DataProvenance }> {
  const invoke = await getInvoke();
  if (!invoke) {
    return {
      output: "Local runtime unavailable in browser preview.",
      provenance: {
        feature: "suggested_reply",
        mode: "local",
        destination: "browser_dev",
        reason: "Tauri runtime not attached",
      },
    };
  }

  return invoke("ai_suggest_reply", {
    payload: {
      subject,
      body,
      mode,
      cloud_provider: mode === "cloud" ? "open_ai" : null,
    },
  });
}

export async function aiCreateTasksFromEmail(
  accountId: string,
  body: string,
  mode: "local" | "cloud",
  listId = "@default"
): Promise<AiTaskExtractionResult> {
  const invoke = await getInvoke();
  if (!invoke) {
    throw new Error("AI action extraction requires the Tauri runtime");
  }

  return invoke("ai_create_tasks_from_email", {
    payload: {
      account_id: accountId,
      list_id: listId,
      body,
      mode,
      cloud_provider: mode === "cloud" ? "open_ai" : null,
    },
  });
}

export async function validateLocalAiRuntime(
  payload: ValidateLocalAiRuntimePayload
): Promise<ValidateLocalAiRuntimeResponse> {
  const invoke = await getInvoke();
  if (!invoke) {
    return {
      valid: false,
      errors: ["Runtime validation requires the Tauri runtime"],
    };
  }

  return invoke("validate_local_ai_runtime", { payload });
}
