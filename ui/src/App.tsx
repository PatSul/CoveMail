import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  aiCreateTasksFromEmail,
  aiSuggestReply,
  aiSummarize,
  beginOAuthPkce,
  bootstrap,
  completeOAuthPkce,
  createTaskFromText,
  exportIcs,
  importIcs,
  listAccounts,
  listMailFolders,
  listMailThreads,
  listTasks,
  listThreadMessages,
  queueEmailSync,
  runSyncQueue,
  saveConfig,
  searchMail,
  sendMail,
  validateLocalAiRuntime,
} from "./lib/api";
import type {
  Account,
  BootstrapResponse,
  DataProvenance,
  MailAddress,
  MailAttachment,
  MailFolder,
  MailMessage,
  MailThreadSummary,
  OutgoingAttachment,
  Provider,
  ReminderTask,
  SyncRunSummary,
} from "./types";

type View = "inbox" | "calendar" | "tasks" | "ai" | "settings";
type ThemeMode = "system" | "aurora" | "midnight" | "linen";
type ResolvedTheme = Exclude<ThemeMode, "system">;
type ToastTone = "info" | "success" | "warning" | "error";

interface ToastItem {
  id: number;
  title: string;
  detail: string;
  tone: ToastTone;
}

const views: Array<{ id: View; label: string; hint: string; icon: string }> = [
  { id: "inbox", label: "Inbox", hint: "Folders + threads", icon: "IN" },
  { id: "calendar", label: "Calendar", hint: "ICS pipelines", icon: "CL" },
  { id: "tasks", label: "Reminders", hint: "Capture + execute", icon: "TK" },
  { id: "ai", label: "AI Studio", hint: "Local-first drafting", icon: "AI" },
  { id: "settings", label: "Privacy", hint: "Control surface", icon: "PV" },
];

const quickQueries = [
  { label: "Unread Today", query: "unread emails from today" },
  { label: "Priority Threads", query: "messages flagged urgent this week" },
  { label: "Calendar Context", query: "emails about meeting invites this month" },
];

const themeOptions: Array<{ id: ThemeMode; label: string; description: string }> = [
  { id: "system", label: "System", description: "Follow your OS preference" },
  { id: "aurora", label: "Aurora", description: "Crisp daylight energy" },
  { id: "midnight", label: "Midnight", description: "Focused low-light command mode" },
  { id: "linen", label: "Linen", description: "Warm editorial workspace" },
];

const oauthProviders: Array<{ id: Provider; label: string }> = [
  { id: "gmail", label: "Google (Gmail)" },
  { id: "outlook", label: "Microsoft 365" },
  { id: "exchange", label: "Exchange" },
];

const THEME_STORAGE_KEY = "covemail.theme_mode";

function isThemeMode(value: string): value is ThemeMode {
  return themeOptions.some((option) => option.id === value);
}

function readStoredThemeMode(): ThemeMode {
  if (typeof window === "undefined") return "system";
  const raw = window.localStorage.getItem(THEME_STORAGE_KEY);
  return raw && isThemeMode(raw) ? raw : "system";
}

function resolveThemeMode(mode: ThemeMode, prefersDark: boolean): ResolvedTheme {
  return mode === "system" ? (prefersDark ? "midnight" : "aurora") : mode;
}

function summarizeSync(summary: SyncRunSummary): string {
  const parts: string[] = [];
  if (summary.email_messages_synced > 0) parts.push(`${summary.email_messages_synced} email items`);
  if (summary.calendar_events_synced > 0) parts.push(`${summary.calendar_events_synced} calendar events`);
  if (summary.tasks_synced > 0) parts.push(`${summary.tasks_synced} tasks`);
  if (summary.retried_jobs > 0) parts.push(`${summary.retried_jobs} retried`);
  if (summary.failed_jobs > 0) parts.push(`${summary.failed_jobs} failed`);
  return parts.length === 0 ? `${summary.completed_jobs} completed jobs` : parts.join(" | ");
}

function senderFromMessage(message: MailMessage): string {
  const from = message.from[0];
  return from ? from.name?.trim() || from.address : "Unknown sender";
}

function formatDateTime(raw: string): string {
  return new Date(raw).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function taskStatusLabel(status: ReminderTask["status"]): string {
  switch (status) {
    case "not_started":
      return "Not started";
    case "in_progress":
      return "In progress";
    case "completed":
      return "Completed";
    case "canceled":
      return "Canceled";
    default:
      return "Unknown";
  }
}

function taskStatusClass(status: ReminderTask["status"]): string {
  return status.replace("_", "-");
}

function themeTagline(theme: ResolvedTheme): string {
  if (theme === "midnight") return "Night focus mode";
  if (theme === "linen") return "Warm studio mode";
  return "Daylight command mode";
}

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  const kb = value / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  return `${(kb / 1024).toFixed(1)} MB`;
}

function parseRecipientList(raw: string): MailAddress[] {
  return raw
    .split(/[;,]/)
    .map((segment) => segment.trim())
    .filter(Boolean)
    .map((address) => ({ address }));
}

function parseOAuthInput(value: string): { code: string | null; state: string | null } {
  const trimmed = value.trim();
  if (!trimmed) return { code: null, state: null };
  if (trimmed.startsWith("http://") || trimmed.startsWith("https://")) {
    try {
      const url = new URL(trimmed);
      return {
        code: url.searchParams.get("code"),
        state: url.searchParams.get("state"),
      };
    } catch {
      return { code: null, state: null };
    }
  }
  return { code: null, state: null };
}

async function toOutgoingAttachment(file: File): Promise<OutgoingAttachment> {
  const contentBase64 = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("failed to read attachment"));
    reader.onload = () => {
      const payload = String(reader.result ?? "");
      resolve(payload.includes(",") ? payload.split(",", 2)[1] : payload);
    };
    reader.readAsDataURL(file);
  });

  return {
    file_name: file.name,
    mime_type: file.type || "application/octet-stream",
    content_base64: contentBase64,
    inline: false,
  };
}

export default function App() {
  const [view, setView] = useState<View>("inbox");
  const [boot, setBoot] = useState<BootstrapResponse | null>(null);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [selectedAccountId, setSelectedAccountId] = useState("");

  const [folders, setFolders] = useState<MailFolder[]>([]);
  const [selectedFolderPath, setSelectedFolderPath] = useState("INBOX");
  const [threads, setThreads] = useState<MailThreadSummary[]>([]);
  const [selectedThreadId, setSelectedThreadId] = useState<string | null>(null);
  const [threadMessages, setThreadMessages] = useState<MailMessage[]>([]);
  const [selectedMessageId, setSelectedMessageId] = useState<string | null>(null);

  const [mailQuery, setMailQuery] = useState("");
  const [mailResults, setMailResults] = useState<MailMessage[]>([]);

  const [composeTo, setComposeTo] = useState("");
  const [composeSubject, setComposeSubject] = useState("");
  const [composeBody, setComposeBody] = useState("");
  const [composeAttachments, setComposeAttachments] = useState<OutgoingAttachment[]>([]);

  const [taskText, setTaskText] = useState("");
  const [tasks, setTasks] = useState<ReminderTask[]>([]);

  const [icsInput, setIcsInput] = useState("BEGIN:VCALENDAR\nVERSION:2.0\nEND:VCALENDAR");
  const [icsOutput, setIcsOutput] = useState("");

  const [aiSubject, setAiSubject] = useState("");
  const [aiBody, setAiBody] = useState("");
  const [aiMode, setAiMode] = useState<"local" | "cloud">("local");
  const [aiOutput, setAiOutput] = useState("");

  const [oauthProvider, setOauthProvider] = useState<Provider>("gmail");
  const [oauthEmailAddress, setOauthEmailAddress] = useState("");
  const [oauthDisplayName, setOauthDisplayName] = useState("");
  const [oauthClientId, setOauthClientId] = useState("");
  const [oauthRedirectUrl, setOauthRedirectUrl] = useState("http://127.0.0.1:8765/oauth/callback");
  const [oauthSessionId, setOauthSessionId] = useState<string | null>(null);
  const [oauthAuthorizationUrl, setOauthAuthorizationUrl] = useState("");
  const [oauthState, setOauthState] = useState("");
  const [oauthCode, setOauthCode] = useState("");
  const [oauthCalendarId, setOauthCalendarId] = useState("primary");
  const [oauthTaskListId, setOauthTaskListId] = useState("@default");
  const [localLlamaBinary, setLocalLlamaBinary] = useState("");
  const [localModelPath, setLocalModelPath] = useState("");
  const [localAiValidationErrors, setLocalAiValidationErrors] = useState<string[]>([]);

  const [provenance, setProvenance] = useState<DataProvenance | null>(null);
  const [status, setStatus] = useState("Ready");
  const [toasts, setToasts] = useState<ToastItem[]>([]);

  const [themeMode, setThemeMode] = useState<ThemeMode>(() => readStoredThemeMode());
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>("aurora");

  const toastCounter = useRef(0);

  const pushToast = useCallback((title: string, detail: string, tone: ToastTone = "info") => {
    const id = ++toastCounter.current;
    setToasts((prev) => [...prev, { id, title, detail, tone }].slice(-4));
    window.setTimeout(() => {
      setToasts((prev) => prev.filter((toast) => toast.id !== id));
    }, 4600);
  }, []);

  const selectedAccount = useMemo(
    () => accounts.find((account) => account.id === selectedAccountId) ?? null,
    [accounts, selectedAccountId]
  );
  const activeView = useMemo(() => views.find((entry) => entry.id === view) ?? views[0], [view]);
  const selectedMessage = useMemo(
    () =>
      threadMessages.find((message) => message.id === selectedMessageId) ??
      threadMessages[threadMessages.length - 1] ??
      null,
    [threadMessages, selectedMessageId]
  );
  const activeFolder = useMemo(
    () => folders.find((folder) => folder.path === selectedFolderPath) ?? null,
    [folders, selectedFolderPath]
  );

  const openTaskCount = useMemo(() => tasks.filter((task) => task.status !== "completed").length, [tasks]);
  const criticalTaskCount = useMemo(
    () =>
      tasks.filter(
        (task) =>
          (task.priority === "critical" || task.priority === "high") &&
          task.status !== "completed"
      ).length,
    [tasks]
  );
  const dueSoonTaskCount = useMemo(() => {
    const now = Date.now();
    const nextDay = now + 24 * 60 * 60 * 1000;
    return tasks.filter((task) => {
      if (!task.due_at || task.status === "completed") return false;
      const due = new Date(task.due_at).getTime();
      return due >= now && due <= nextDay;
    }).length;
  }, [tasks]);

  useEffect(() => {
    const root = document.documentElement;
    const platform = /mac|iphone|ipad|ipod/i.test(navigator.userAgent) ? "macos" : "default";
    root.dataset.platform = platform;
    return () => {
      delete root.dataset.platform;
    };
  }, []);

  useEffect(() => {
    if (typeof window === "undefined") return;

    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const applyTheme = () => {
      const nextResolved = resolveThemeMode(themeMode, media.matches);
      const root = document.documentElement;
      root.dataset.theme = nextResolved;
      root.dataset.themeMode = themeMode;
      root.style.colorScheme = nextResolved === "midnight" ? "dark" : "light";
      setResolvedTheme(nextResolved);
      window.localStorage.setItem(THEME_STORAGE_KEY, themeMode);
    };

    applyTheme();
    if (themeMode !== "system") return;

    const handle = () => applyTheme();
    if (typeof media.addEventListener === "function") {
      media.addEventListener("change", handle);
      return () => media.removeEventListener("change", handle);
    }

    media.addListener(handle);
    return () => media.removeListener(handle);
  }, [themeMode]);

  const loadFolderState = useCallback(async (accountId: string, refreshRemote: boolean) => {
    const nextFolders = await listMailFolders(accountId, refreshRemote);
    setFolders(nextFolders);
    setSelectedFolderPath((previous) => {
      if (previous && nextFolders.some((folder) => folder.path === previous)) {
        return previous;
      }
      const inbox = nextFolders.find((folder) => folder.path.toUpperCase() === "INBOX");
      return inbox?.path ?? nextFolders[0]?.path ?? "INBOX";
    });
  }, []);

  useEffect(() => {
    setStatus("Loading account context...");
    void bootstrap()
      .then((result) => {
        setBoot(result);
        setAccounts(result.accounts);
        if (result.accounts[0]) setSelectedAccountId(result.accounts[0].id);
        setLocalLlamaBinary(result.config.ai.local.llama_cpp_binary ?? "");
        setLocalModelPath(result.config.ai.local.model_path ?? "");
        setAiMode(result.config.privacy.default_ai_mode);
        setStatus("Ready");
      })
      .catch((error: unknown) => {
        const message = String(error);
        setStatus(message);
        pushToast("Bootstrap failed", message, "error");
      });
  }, [pushToast]);

  useEffect(() => {
    if (!selectedAccountId) {
      setFolders([]);
      setThreads([]);
      setThreadMessages([]);
      return;
    }

    void listTasks(selectedAccountId)
      .then(setTasks)
      .catch((error: unknown) => {
        const message = String(error);
        setStatus(message);
        pushToast("Task sync failed", message, "error");
      });

    void loadFolderState(selectedAccountId, true).catch((error: unknown) => {
      const message = String(error);
      setStatus(message);
      pushToast("Folder sync failed", message, "error");
    });
  }, [selectedAccountId, loadFolderState, pushToast]);

  useEffect(() => {
    if (!selectedAccountId || !selectedFolderPath) {
      setThreads([]);
      return;
    }

    void listMailThreads(selectedAccountId, selectedFolderPath)
      .then((result) => {
        setThreads(result);
        if (result.length === 0) {
          setSelectedThreadId(null);
          setThreadMessages([]);
          setSelectedMessageId(null);
          return;
        }
        setSelectedThreadId((previous) => {
          if (previous && result.some((thread) => thread.thread_id === previous)) {
            return previous;
          }
          return result[0].thread_id;
        });
      })
      .catch((error: unknown) => {
        const message = String(error);
        setStatus(message);
        pushToast("Thread load failed", message, "error");
      });
  }, [selectedAccountId, selectedFolderPath, pushToast]);

  useEffect(() => {
    if (!selectedAccountId || !selectedThreadId) {
      setThreadMessages([]);
      setSelectedMessageId(null);
      return;
    }

    void listThreadMessages(selectedAccountId, selectedThreadId)
      .then((messages) => {
        setThreadMessages(messages);
        setSelectedMessageId(messages[messages.length - 1]?.id ?? null);
      })
      .catch((error: unknown) => {
        const message = String(error);
        setStatus(message);
        pushToast("Message load failed", message, "error");
      });
  }, [selectedAccountId, selectedThreadId, pushToast]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let active = true;

    void (async () => {
      try {
        const event = await import("@tauri-apps/api/event");
        const off = await event.listen<SyncRunSummary>("sync://summary", ({ payload }) => {
          const syncMessage = summarizeSync(payload);
          setStatus(`Background sync: ${syncMessage}`);
          pushToast(
            payload.failed_jobs > 0 ? "Sync has issues" : "Sync complete",
            syncMessage,
            payload.failed_jobs > 0 ? "warning" : "success"
          );
        });

        if (!active) {
          off();
          return;
        }
        unlisten = off;
      } catch {
        // Browser preview mode does not attach Tauri events.
      }
    })();

    return () => {
      active = false;
      if (unlisten) unlisten();
    };
  }, [pushToast]);

  async function executeMailSearch(rawQuery: string) {
    const query = rawQuery.trim();
    if (!query) return;

    setStatus("Searching local index...");
    setMailQuery(query);
    try {
      const result = await searchMail(query);
      const filtered = selectedAccountId
        ? result.items.filter((item) => item.account_id === selectedAccountId)
        : result.items;
      setMailResults(filtered);
      setStatus(`Search returned ${filtered.length} messages`);
      pushToast("Search complete", `${filtered.length} matches`, "success");
    } catch (error) {
      const message = String(error);
      setStatus(message);
      pushToast("Search failed", message, "error");
    }
  }

  async function onRunSync() {
    if (!selectedAccountId) return;

    setStatus("Queueing + running sync jobs...");
    try {
      await queueEmailSync(selectedAccountId);
      const summary = await runSyncQueue();
      const message = summarizeSync(summary);
      setStatus(`Sync complete: ${message}`);
      pushToast(
        summary.failed_jobs > 0 ? "Sync has issues" : "Sync complete",
        message,
        summary.failed_jobs > 0 ? "warning" : "success"
      );
      await loadFolderState(selectedAccountId, false);
    } catch (error) {
      const message = String(error);
      setStatus(message);
      pushToast("Sync failed", message, "error");
    }
  }

  async function onCreateTask() {
    if (!selectedAccountId || !taskText.trim()) return;

    try {
      const task = await createTaskFromText(selectedAccountId, taskText);
      setTasks((prev) => [task, ...prev]);
      setTaskText("");
      setStatus("Task created");
      pushToast("Task created", task.title, "success");
    } catch (error) {
      const message = String(error);
      setStatus(message);
      pushToast("Task creation failed", message, "error");
    }
  }

  async function onImportIcs() {
    if (!selectedAccountId) return;
    try {
      const count = await importIcs(selectedAccountId, icsInput);
      setStatus(`Imported ${count} event(s)`);
      pushToast("Calendar import complete", `${count} events`, "success");
    } catch (error) {
      const message = String(error);
      setStatus(message);
      pushToast("Calendar import failed", message, "error");
    }
  }

  async function onExportIcs() {
    if (!selectedAccountId) return;
    try {
      const payload = await exportIcs(selectedAccountId);
      setIcsOutput(payload);
      setStatus("Calendar exported to ICS");
      pushToast("Calendar export complete", "ICS ready for download/share", "success");
    } catch (error) {
      const message = String(error);
      setStatus(message);
      pushToast("Calendar export failed", message, "error");
    }
  }

  async function onAiSummarize() {
    try {
      const response = await aiSummarize(aiSubject, aiBody, aiMode);
      setAiOutput(response.output);
      setProvenance(response.provenance);
      setStatus("Summary ready");
      pushToast("AI summary ready", `${response.provenance.mode} mode`, "success");
    } catch (error) {
      const message = String(error);
      setStatus(message);
      pushToast("AI summary failed", message, "error");
    }
  }

  async function onAiReply() {
    try {
      const response = await aiSuggestReply(aiSubject, aiBody, aiMode);
      setAiOutput(response.output);
      setProvenance(response.provenance);
      setStatus("Draft reply generated");
      pushToast("Draft reply ready", "Review before sending", "success");
    } catch (error) {
      const message = String(error);
      setStatus(message);
      pushToast("Reply generation failed", message, "error");
    }
  }

  async function onAiCreateTasks() {
    if (!selectedAccountId) {
      pushToast("No account selected", "Select an account before creating tasks", "warning");
      return;
    }

    const sourceBody = aiBody.trim() || selectedMessage?.body_text || selectedMessage?.preview || "";
    if (!sourceBody) {
      pushToast("Missing source text", "Provide an email body to extract action items", "warning");
      return;
    }

    try {
      const response = await aiCreateTasksFromEmail(selectedAccountId, sourceBody, aiMode);
      setTasks((previous) => {
        const byId = new Map(previous.map((task) => [task.id, task]));
        for (const task of response.created) byId.set(task.id, task);
        return Array.from(byId.values());
      });
      setProvenance(response.provenance);
      setStatus(`Created ${response.created.length} task(s) from AI extraction`);
      pushToast("Tasks created", `${response.created.length} tasks added`, "success");
    } catch (error) {
      const message = String(error);
      setStatus(message);
      pushToast("Task extraction failed", message, "error");
    }
  }

  async function onComposeFiles(event: React.ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.target.files ?? []);
    if (!files.length) return;

    try {
      const attachments = await Promise.all(files.map((file) => toOutgoingAttachment(file)));
      setComposeAttachments((prev) => [...prev, ...attachments]);
      pushToast("Attachment queued", `${attachments.length} file(s) ready`, "success");
    } catch (error) {
      pushToast("Attachment read failed", String(error), "error");
    } finally {
      event.target.value = "";
    }
  }

  async function onSendMail() {
    if (!selectedAccountId || !selectedAccount) return;

    const to = parseRecipientList(composeTo);
    if (to.length === 0) {
      pushToast("Missing recipients", "Add at least one To address", "warning");
      return;
    }

    try {
      await sendMail(selectedAccountId, {
        from: {
          name: selectedAccount.display_name,
          address: selectedAccount.email_address,
        },
        to,
        cc: [],
        bcc: [],
        reply_to: [],
        subject: composeSubject,
        body_text: composeBody,
        body_html: null,
        attachments: composeAttachments,
      });

      setComposeSubject("");
      setComposeBody("");
      setComposeAttachments([]);
      setStatus("Draft sent");
      pushToast("Draft sent", "Message submitted to provider", "success");
    } catch (error) {
      const message = String(error);
      setStatus(message);
      pushToast("Send failed", message, "error");
    }
  }

  async function onBeginOAuth() {
    if (!oauthEmailAddress.trim() || !oauthClientId.trim() || !oauthRedirectUrl.trim()) {
      pushToast("Missing OAuth fields", "Email, Client ID, and Redirect URL are required", "warning");
      return;
    }

    try {
      const response = await beginOAuthPkce({
        provider: oauthProvider,
        email_address: oauthEmailAddress.trim(),
        display_name: oauthDisplayName.trim() || null,
        client_id: oauthClientId.trim(),
        redirect_url: oauthRedirectUrl.trim(),
      });

      setOauthSessionId(response.session_id);
      setOauthAuthorizationUrl(response.authorization_url);
      setStatus("OAuth session created");
      pushToast("OAuth started", "Authorize in browser, then paste callback URL or code", "success");
      window.open(response.authorization_url, "_blank", "noopener,noreferrer");
    } catch (error) {
      const message = String(error);
      setStatus(message);
      pushToast("OAuth start failed", message, "error");
    }
  }

  async function onCompleteOAuth() {
    if (!oauthSessionId || !oauthCode.trim() || !oauthState.trim()) {
      pushToast("Missing OAuth completion fields", "Session, state, and code are required", "warning");
      return;
    }

    try {
      const response = await completeOAuthPkce({
        session_id: oauthSessionId,
        csrf_state: oauthState.trim(),
        code: oauthCode.trim(),
        calendar_id: oauthCalendarId.trim() || null,
        task_list_id: oauthTaskListId.trim() || null,
      });

      const refreshedAccounts = await listAccounts();
      setAccounts(refreshedAccounts);
      setSelectedAccountId(response.account.id);
      setOauthSessionId(null);
      setOauthAuthorizationUrl("");
      setOauthCode("");
      setStatus("OAuth account added");
      pushToast("Account linked", response.account.email_address, "success");
    } catch (error) {
      const message = String(error);
      setStatus(message);
      pushToast("OAuth completion failed", message, "error");
    }
  }

  async function onValidateLocalAiRuntime() {
    try {
      const result = await validateLocalAiRuntime({
        llama_cpp_binary: localLlamaBinary.trim() || null,
        model_path: localModelPath.trim() || null,
      });
      setLocalAiValidationErrors(result.errors);
      if (result.valid) {
        pushToast("Local runtime valid", "llama.cpp and GGUF model paths look good", "success");
      } else {
        pushToast("Runtime validation failed", result.errors.join(" | "), "warning");
      }
    } catch (error) {
      const message = String(error);
      setLocalAiValidationErrors([message]);
      pushToast("Validation failed", message, "error");
    }
  }

  async function onSaveLocalAiRuntime() {
    if (!boot) return;

    const nextConfig = {
      ...boot.config,
      ai: {
        ...boot.config.ai,
        local: {
          ...boot.config.ai.local,
          llama_cpp_binary: localLlamaBinary.trim() || null,
          model_path: localModelPath.trim() || null,
        },
      },
    };

    try {
      await saveConfig(nextConfig);
      setBoot((previous) =>
        previous
          ? {
              ...previous,
              config: nextConfig,
            }
          : previous
      );
      pushToast("AI runtime saved", "Local runtime configuration stored", "success");
    } catch (error) {
      const message = String(error);
      setStatus(message);
      pushToast("Save failed", message, "error");
    }
  }

  return (
    <div className="app-shell">
      <span className="ambient-orb orb-a" aria-hidden />
      <span className="ambient-orb orb-b" aria-hidden />

      <aside className="side-rail">
        <div className="brand">
          <span className="brand-badge">AE</span>
          <div className="brand-copy">
            <h1>Cove Mail</h1>
            <p>Privacy-first personal command center</p>
          </div>
        </div>

        <nav className="view-nav">
          {views.map((entry) => (
            <button
              key={entry.id}
              className={entry.id === view ? "active nav-entry" : "nav-entry"}
              onClick={() => setView(entry.id)}
            >
              <span className="nav-main">
                <span className="nav-icon">{entry.icon}</span>
                <span>{entry.label}</span>
              </span>
              <small>{entry.hint}</small>
            </button>
          ))}
        </nav>

        <section className="account-panel">
          <header>
            <h2>Active Account</h2>
            <p>{accounts.length} configured</p>
          </header>

          <select
            value={selectedAccountId}
            onChange={(event) => setSelectedAccountId(event.target.value)}
            disabled={!accounts.length}
          >
            {accounts.length === 0 && <option value="">No accounts configured</option>}
            {accounts.map((account) => (
              <option key={account.id} value={account.id}>
                {account.display_name} ({account.email_address})
              </option>
            ))}
          </select>

          <dl>
            <div>
              <dt>Pending sync</dt>
              <dd>{boot?.pending_sync_jobs ?? 0}</dd>
            </div>
            <div>
              <dt>Default AI mode</dt>
              <dd>{boot?.config.privacy.default_ai_mode ?? "local"}</dd>
            </div>
            <div>
              <dt>Theme mood</dt>
              <dd>{themeTagline(resolvedTheme)}</dd>
            </div>
          </dl>
        </section>

        <section className="privacy-card">
          <strong>Security baseline</strong>
          <p>Keychain secrets, local-first processing, and explicit cloud opt-in.</p>
        </section>
      </aside>

      <main className="workspace">
        <header className="workspace-header">
          <div>
            <h2>{activeView.label}</h2>
            <p>{selectedAccount?.email_address ?? "No account selected"}</p>
          </div>
          <div className="header-actions">
            <button className="btn-primary" onClick={onRunSync}>
              Run Sync
            </button>
            <span className="status-pill">{status}</span>
          </div>
        </header>

        {provenance && (
          <section className="provenance-banner">
            <strong>Data provenance:</strong> {provenance.feature} via {provenance.destination} (
            {provenance.reason})
          </section>
        )}

        <section className="command-ribbon">
          <p className="kicker">Fast lane</p>
          <div className="quick-query-list">
            {quickQueries.map((item) => (
              <button key={item.label} className="quick-chip" onClick={() => void executeMailSearch(item.query)}>
                {item.label}
              </button>
            ))}
          </div>
        </section>

        <section className="metric-grid">
          <article className="metric-card">
            <h3>Indexed Messages</h3>
            <p>{mailResults.length}</p>
          </article>
          <article className="metric-card">
            <h3>Open Reminders</h3>
            <p>{openTaskCount}</p>
          </article>
          <article className="metric-card">
            <h3>Critical Queue</h3>
            <p>{criticalTaskCount}</p>
          </article>
          <article className="metric-card">
            <h3>Due in 24h</h3>
            <p>{dueSoonTaskCount}</p>
          </article>
        </section>

        {view === "inbox" && (
          <section className="inbox-layout">
            <article className="card folder-card">
              <header className="card-head">
                <h3>Folders</h3>
                <button
                  onClick={() => {
                    if (!selectedAccountId) return;
                    void loadFolderState(selectedAccountId, true);
                  }}
                >
                  Refresh
                </button>
              </header>

              <div className="row">
                <input
                  value={mailQuery}
                  onChange={(event) => setMailQuery(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") void executeMailSearch(mailQuery);
                  }}
                  placeholder="Search local index"
                />
                <button onClick={() => void executeMailSearch(mailQuery)}>Search</button>
              </div>

              <div className="folder-list">
                {folders.map((folder) => (
                  <button
                    key={folder.path}
                    className={selectedFolderPath === folder.path ? "folder-item selected" : "folder-item"}
                    onClick={() => setSelectedFolderPath(folder.path)}
                  >
                    <span>{folder.path}</span>
                    <small>{folder.unread_count}/{folder.total_count}</small>
                  </button>
                ))}
                {folders.length === 0 && <p className="muted">No folders cached yet. Run sync.</p>}
              </div>

              <div className="search-hit-list">
                {mailResults.slice(0, 8).map((message) => (
                  <button
                    key={message.id}
                    className="search-hit"
                    onClick={() => {
                      setSelectedFolderPath(message.folder_path);
                      setSelectedThreadId(message.thread_id);
                    }}
                  >
                    <strong>{message.subject || "(No subject)"}</strong>
                    <span>{senderFromMessage(message)}</span>
                  </button>
                ))}
              </div>
            </article>

            <article className="card thread-list-card list-card">
              <h3>{activeFolder ? `Threads · ${activeFolder.path}` : "Threads"}</h3>
              <div className="thread-list">
                {threads.map((thread) => (
                  <button
                    key={thread.thread_id}
                    className={selectedThreadId === thread.thread_id ? "thread-item selected" : "thread-item"}
                    onClick={() => setSelectedThreadId(thread.thread_id)}
                  >
                    <div className="mail-topline">
                      <strong>{thread.subject || "(No subject)"}</strong>
                      <time>{formatDateTime(thread.most_recent_at)}</time>
                    </div>
                    <span>{thread.participants.slice(0, 2).join(", ") || "No participants"}</span>
                    <span>{thread.unread_count} unread · {thread.message_count} messages</span>
                  </button>
                ))}
                {threads.length === 0 && <p className="muted">No threads in this folder yet.</p>}
              </div>
            </article>

            <article className="card message-preview-card">
              {selectedMessage ? (
                <>
                  <header className="message-preview-head">
                    <h3>{selectedMessage.subject || "(No subject)"}</h3>
                    <span className="info-pill">{selectedMessage.folder_path}</span>
                  </header>
                  <dl className="meta-list">
                    <div>
                      <dt>From</dt>
                      <dd>{senderFromMessage(selectedMessage)}</dd>
                    </div>
                    <div>
                      <dt>Received</dt>
                      <dd>{formatDateTime(selectedMessage.received_at)}</dd>
                    </div>
                    <div>
                      <dt>Thread</dt>
                      <dd>{selectedMessage.thread_id.slice(0, 20)}...</dd>
                    </div>
                  </dl>

                  {selectedMessage.attachments.length > 0 && (
                    <div className="attachment-list">
                      {selectedMessage.attachments.map((attachment: MailAttachment) => (
                        <div key={attachment.id} className="attachment-chip">
                          <strong>{attachment.file_name}</strong>
                          <small>{formatBytes(attachment.size)}</small>
                        </div>
                      ))}
                    </div>
                  )}

                  <pre className="message-body">{selectedMessage.body_text || selectedMessage.preview || "No body available."}</pre>
                </>
              ) : (
                <div className="empty-state">
                  <h3>Message Preview</h3>
                  <p>Select a thread to inspect full message details.</p>
                </div>
              )}

              <section className="compose-panel">
                <h3>Compose</h3>
                <input
                  value={composeTo}
                  onChange={(event) => setComposeTo(event.target.value)}
                  placeholder="to@example.com, other@example.com"
                />
                <input
                  value={composeSubject}
                  onChange={(event) => setComposeSubject(event.target.value)}
                  placeholder="Subject"
                />
                <textarea
                  value={composeBody}
                  onChange={(event) => setComposeBody(event.target.value)}
                  placeholder="Write your draft"
                />
                <label className="file-input">
                  <span>Add attachments</span>
                  <input type="file" multiple onChange={onComposeFiles} />
                </label>
                {composeAttachments.length > 0 && (
                  <div className="compose-attachments">
                    {composeAttachments.map((attachment, index) => (
                      <button
                        key={`${attachment.file_name}-${index}`}
                        className="attachment-chip"
                        onClick={() =>
                          setComposeAttachments((previous) => previous.filter((_, itemIndex) => itemIndex !== index))
                        }
                      >
                        <strong>{attachment.file_name}</strong>
                        <small>remove</small>
                      </button>
                    ))}
                  </div>
                )}
                <button className="btn-primary" onClick={onSendMail}>Send Draft</button>
              </section>
            </article>
          </section>
        )}

        {view === "calendar" && (
          <section className="panel-grid">
            <article className="card">
              <h3>ICS Import Lab</h3>
              <p className="muted">Drop or paste iCalendar payload to hydrate local cache.</p>
              <textarea value={icsInput} onChange={(event) => setIcsInput(event.target.value)} />
              <button onClick={onImportIcs}>Import ICS</button>
            </article>

            <article className="card">
              <h3>ICS Export Lab</h3>
              <p className="muted">Export your next 12 months to a portable iCalendar file.</p>
              <button onClick={onExportIcs}>Export Next Year</button>
              <textarea value={icsOutput} onChange={(event) => setIcsOutput(event.target.value)} />
            </article>
          </section>
        )}

        {view === "tasks" && (
          <section className="panel-grid">
            <article className="card">
              <h3>Quick Capture</h3>
              <div className="row">
                <input
                  value={taskText}
                  onChange={(event) => setTaskText(event.target.value)}
                  placeholder="Call vendor tomorrow high notes: ask for revised quote"
                />
                <button onClick={onCreateTask}>Create</button>
              </div>
              <p className="muted">Supports due hints, priorities, repeat language, and snooze phrases.</p>
            </article>

            <article className="card list-card">
              <h3>Task Flight Deck</h3>
              <ul className="task-list">
                {tasks.map((task) => (
                  <li key={task.id} className={`task-item priority-${task.priority}`}>
                    <div className="task-topline">
                      <strong>{task.title}</strong>
                      <span className={`priority-chip priority-${task.priority}`}>{task.priority}</span>
                    </div>
                    <div className="task-subline">
                      <span className={`status-chip status-${taskStatusClass(task.status)}`}>
                        {taskStatusLabel(task.status)}
                      </span>
                      {task.due_at ? <time>Due {formatDateTime(task.due_at)}</time> : <span>No due date</span>}
                    </div>
                  </li>
                ))}
                {tasks.length === 0 && <p className="muted">No tasks for this account.</p>}
              </ul>
            </article>
          </section>
        )}

        {view === "ai" && (
          <section className="panel-grid">
            <article className="card">
              <h3>Email Intelligence</h3>
              <label>
                Subject
                <input value={aiSubject} onChange={(event) => setAiSubject(event.target.value)} />
              </label>
              <label>
                Body
                <textarea value={aiBody} onChange={(event) => setAiBody(event.target.value)} />
              </label>
              <div className="mode-switch">
                <button className={aiMode === "local" ? "active" : ""} onClick={() => setAiMode("local")}>
                  Local runtime
                </button>
                <button className={aiMode === "cloud" ? "active" : ""} onClick={() => setAiMode("cloud")}>
                  Cloud opt-in
                </button>
              </div>
              <div className="row">
                <button onClick={onAiSummarize}>Summarize</button>
                <button onClick={onAiReply}>Draft Reply</button>
                <button onClick={onAiCreateTasks}>Create Tasks</button>
              </div>
            </article>

            <article className="card list-card">
              <h3>Output</h3>
              <pre>{aiOutput || "No output yet."}</pre>
              <p className="muted">Suggested replies are drafts and are never auto-sent.</p>
            </article>
          </section>
        )}

        {view === "settings" && (
          <section className="panel-grid settings-grid">
            <article className="card">
              <h3>Privacy Defaults</h3>
              <ul className="flat-list">
                <li>No telemetry or analytics by default</li>
                <li>Credentials stored in OS keychain</li>
                <li>Email/calendar/task cache remains local in SQLite</li>
                <li>Cloud AI is explicit per-feature opt-in</li>
              </ul>

              <label className="settings-field">
                Theme mode
                <select value={themeMode} onChange={(event) => setThemeMode(event.target.value as ThemeMode)}>
                  {themeOptions.map((option) => (
                    <option key={option.id} value={option.id}>{option.label}</option>
                  ))}
                </select>
              </label>

              <div className="theme-grid">
                {themeOptions.map((option) => (
                  <button
                    key={option.id}
                    className={themeMode === option.id ? "theme-tile active" : "theme-tile"}
                    onClick={() => setThemeMode(option.id)}
                  >
                    <span className={`theme-swatch swatch-${option.id}`} />
                    <span>
                      <strong>{option.label}</strong>
                      <small>{option.description}</small>
                    </span>
                  </button>
                ))}
              </div>
              <p className="muted">Current palette: {resolvedTheme}</p>
            </article>

            <article className="card">
              <h3>Local AI Runtime</h3>
              <p className="muted">All local inference stays on-device unless cloud mode is explicitly enabled.</p>

              <label>
                llama.cpp binary path
                <input
                  value={localLlamaBinary}
                  onChange={(event) => setLocalLlamaBinary(event.target.value)}
                  placeholder="C:\\tools\\llama\\llama-server.exe"
                />
              </label>

              <label>
                GGUF model path
                <input
                  value={localModelPath}
                  onChange={(event) => setLocalModelPath(event.target.value)}
                  placeholder="D:\\models\\mistral-7b-instruct.Q4_K_M.gguf"
                />
              </label>

              <div className="row">
                <button onClick={onValidateLocalAiRuntime}>Validate Runtime</button>
                <button className="btn-primary" onClick={onSaveLocalAiRuntime}>Save Runtime</button>
              </div>

              {localAiValidationErrors.length > 0 && (
                <ul className="flat-list">
                  {localAiValidationErrors.map((error, index) => (
                    <li key={`${error}-${index}`}>{error}</li>
                  ))}
                </ul>
              )}
            </article>

            <article className="card oauth-card">
              <h3>Account Onboarding (OAuth PKCE)</h3>
              <p className="muted">Data goes only to selected providers. Tokens are stored in OS keychain.</p>

              <label>
                Provider
                <select value={oauthProvider} onChange={(event) => setOauthProvider(event.target.value as Provider)}>
                  {oauthProviders.map((provider) => (
                    <option key={provider.id} value={provider.id}>{provider.label}</option>
                  ))}
                </select>
              </label>

              <label>
                Email address
                <input value={oauthEmailAddress} onChange={(event) => setOauthEmailAddress(event.target.value)} placeholder="name@company.com" />
              </label>

              <label>
                Display name
                <input value={oauthDisplayName} onChange={(event) => setOauthDisplayName(event.target.value)} placeholder="Pat" />
              </label>

              <label>
                OAuth client ID
                <input value={oauthClientId} onChange={(event) => setOauthClientId(event.target.value)} placeholder="provider app client id" />
              </label>

              <label>
                Redirect URL
                <input value={oauthRedirectUrl} onChange={(event) => setOauthRedirectUrl(event.target.value)} />
              </label>

              <div className="row">
                <button onClick={onBeginOAuth}>Begin OAuth</button>
                {oauthAuthorizationUrl && (
                  <button onClick={() => window.open(oauthAuthorizationUrl, "_blank", "noopener,noreferrer")}>Open Authorization URL</button>
                )}
              </div>

              {oauthAuthorizationUrl && (
                <div className="oauth-url-box">
                  <strong>Authorization URL</strong>
                  <code>{oauthAuthorizationUrl}</code>
                </div>
              )}

              <label>
                Callback URL or raw code
                <textarea
                  value={oauthCode}
                  onChange={(event) => {
                    const value = event.target.value;
                    const parsed = parseOAuthInput(value);
                    setOauthCode(parsed.code ?? value);
                    if (parsed.state) setOauthState(parsed.state);
                  }}
                  placeholder="Paste redirected URL, or just the code"
                />
              </label>

              <label>
                CSRF state
                <input value={oauthState} onChange={(event) => setOauthState(event.target.value)} placeholder="state from callback" />
              </label>

              <label>
                Calendar ID
                <input value={oauthCalendarId} onChange={(event) => setOauthCalendarId(event.target.value)} placeholder="primary" />
              </label>

              <label>
                Task list ID
                <input value={oauthTaskListId} onChange={(event) => setOauthTaskListId(event.target.value)} placeholder="@default" />
              </label>

              <button className="btn-primary" onClick={onCompleteOAuth} disabled={!oauthSessionId}>Complete OAuth</button>
              <p className="muted">Active session: {oauthSessionId ? oauthSessionId.slice(0, 12) + "..." : "none"}</p>
            </article>
          </section>
        )}
      </main>

      <section className="toast-stack" aria-live="polite">
        {toasts.map((toast) => (
          <article key={toast.id} className={`toast toast-${toast.tone}`}>
            <h4>{toast.title}</h4>
            <p>{toast.detail}</p>
          </article>
        ))}
      </section>
    </div>
  );
}
