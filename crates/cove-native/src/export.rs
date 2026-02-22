use cove_config::AppConfig;
use cove_core::Account;
use age::Encryptor;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Serialize, Deserialize)]
pub struct AccountExport {
    pub account: Account,
    pub protocol_settings_json: Option<serde_json::Value>,
    pub secrets: BTreeMap<String, String>, // mapping namespace -> secret value
}

#[derive(Serialize, Deserialize)]
pub struct ExportPayload {
    pub config: AppConfig,
    pub sqlcipher_key: Option<String>,
    pub accounts: Vec<AccountExport>,
}

pub fn export_settings(
    payload: &ExportPayload,
    password: &str,
    path: &Path,
) -> anyhow::Result<()> {
    // Serialize to JSON bytes
    let json_bytes = serde_json::to_vec(payload)?;

    // Encrypt with age
    let encryptor = Encryptor::with_user_passphrase(age::secrecy::SecretString::new(password.to_string()));
    let mut encrypted_file = std::fs::File::create(path)?;
    let mut writer = encryptor.wrap_output(&mut encrypted_file)?;
    writer.write_all(&json_bytes)?;
    writer.finish()?;

    Ok(())
}

pub fn import_settings(password: &str, path: &Path) -> anyhow::Result<ExportPayload> {
    // the password doesn't matter for the file existence, but Decryptor needs to read it
    let encrypted_file = std::fs::File::open(path)?;
    let buffered_file = std::io::BufReader::new(encrypted_file);

    // Read header and decrypt
    let decryptor = match age::Decryptor::new_buffered(buffered_file)? {
        age::Decryptor::Passphrase(d) => d,
        _ => anyhow::bail!("Invalid encryption format: expected passphrase"),
    };

    let mut reader = decryptor.decrypt(&age::secrecy::SecretString::new(password.to_string()), None).map_err(|e| anyhow::anyhow!("Decryption failed: {}", e))?;
    let mut json_bytes = Vec::new();
    reader.read_to_end(&mut json_bytes)?;

    let payload: ExportPayload = serde_json::from_slice(&json_bytes)?;
    Ok(payload)
}
