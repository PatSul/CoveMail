mod commands;
mod state;

use state::AppState;
use tauri::Manager;
use tokio::time::{sleep, Duration};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}

fn main() {
    init_tracing();

    let app_state = tauri::async_runtime::block_on(AppState::initialize())
        .expect("failed to initialize AegisInbox app state");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .manage(app_state)
        .setup(|app| {
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                background_sync_loop(app_handle).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::get_config,
            commands::save_config,
            commands::list_accounts,
            commands::save_account,
            commands::delete_account,
            commands::set_secret,
            commands::begin_oauth_pkce,
            commands::complete_oauth_pkce,
            commands::set_ai_api_key,
            commands::queue_sync_job,
            commands::run_sync_queue,
            commands::search_mail,
            commands::list_mail,
            commands::list_mail_folders,
            commands::list_mail_threads,
            commands::list_thread_messages,
            commands::get_mail_message,
            commands::send_mail,
            commands::list_tasks,
            commands::create_task_from_text,
            commands::import_calendar_ics,
            commands::export_calendar_ics,
            commands::ai_summarize_email,
            commands::ai_suggest_reply,
            commands::ai_extract_action_items,
            commands::ai_create_tasks_from_email,
            commands::validate_local_ai_runtime,
            commands::ai_fetch_available_models,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AegisInbox");
}

async fn background_sync_loop(app_handle: tauri::AppHandle) {
    let mut tick = 0_u64;

    loop {
        {
            let state = app_handle.state::<AppState>();
            if let Err(err) = state.schedule_sync_jobs().await {
                tracing::error!("sync scheduler error: {err}");
            }
        }

        match commands::run_sync_queue(app_handle.state::<AppState>(), app_handle.clone()).await {
            Ok(summary) if summary.completed_jobs > 0 || summary.failed_jobs > 0 => {
                tracing::info!(
                    completed = summary.completed_jobs,
                    failed = summary.failed_jobs,
                    retried = summary.retried_jobs,
                    "background sync cycle completed"
                );
            }
            Ok(_) => {}
            Err(err) => tracing::error!("background sync run failed: {err}"),
        }

        if tick % 12 == 0 {
            let state = app_handle.state::<AppState>();
            if let Err(err) = state.prime_idle_listeners().await {
                tracing::warn!("idle listener refresh failed: {err}");
            }
        }

        tick = tick.wrapping_add(1);
        sleep(Duration::from_secs(15)).await;
    }
}
