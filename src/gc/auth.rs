//! Local Steam session recovery: read the `local.vdf` `ConnectCache` blob and decrypt
//! the refresh-token JWT (AES on Linux/macOS, DPAPI on Windows).
//! The token is a live account credential: in-memory only, never logged/persisted/sent.

use std::path::{Path, PathBuf};

use base64::Engine;
use keyvalues_parser::{Value, Vdf};
use tracing::warn;

use super::error::GcError;

/// A recovered Steam session for one account. `refresh_token` is a live credential:
/// in-memory only, never logged/persisted/sent (custom `Debug` keeps it out of logs).
#[derive(Clone)]
pub(crate) struct AuthContext {
    pub(crate) account_name: String,
    pub(crate) steam_id64: u64,
    pub(crate) refresh_token: String,
}

impl AuthContext {
    /// The 32-bit Steam3 account id deadlock-api expects, not the `SteamID64`.
    pub(crate) fn account_id(&self) -> u32 {
        (self.steam_id64 & 0xFFFF_FFFF) as u32
    }
}

impl core::fmt::Debug for AuthContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AuthContext")
            .field("account_name", &self.account_name)
            .field("steam_id64", &self.steam_id64)
            .field("refresh_token", &"<redacted>")
            .finish()
    }
}

fn err(msg: impl Into<String>) -> GcError {
    GcError::AuthUnavailable(msg.into())
}

fn child<'a>(value: &'a Value<'a>, key: &str) -> Option<&'a Value<'a>> {
    value.get_obj()?.get(key)?.first()
}

// The auth file location differs on Windows.
#[cfg(windows)]
fn local_vdf_path(_steam_dir: &Path) -> Result<PathBuf, GcError> {
    dirs::data_local_dir()
        .map(|d| d.join("Steam").join("local.vdf"))
        .ok_or_else(|| err("could not resolve %LOCALAPPDATA%"))
}

// Result-wrapped to match the Windows variant, which can fail resolving %LOCALAPPDATA%.
#[cfg(not(windows))]
#[allow(clippy::unnecessary_wraps)]
fn local_vdf_path(steam_dir: &Path) -> Result<PathBuf, GcError> {
    Ok(steam_dir.join("local.vdf"))
}

fn all_account_ids(steam_dir: &Path) -> Result<Vec<(u64, String)>, GcError> {
    let path = steam_dir.join("config").join("loginusers.vdf");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| err(format!("cannot read loginusers.vdf: {e}")))?;
    let vdf = keyvalues_parser::parse(&text)
        .map(Vdf::from)
        .map_err(|e| err(format!("cannot parse loginusers.vdf: {e}")))?;
    let users = vdf
        .value
        .get_obj()
        .ok_or_else(|| err("loginusers.vdf has no users"))?;

    let mut accounts = Vec::new();
    for (key, entries) in users.iter() {
        let Ok(steam_id64) = key.parse::<u64>() else {
            warn!("skipping loginusers.vdf block with non-numeric key: {key}");
            continue;
        };
        let Some(name) = entries
            .first()
            .and_then(|user| child(user, "AccountName"))
            .and_then(Value::get_str)
        else {
            warn!("skipping loginusers.vdf block {steam_id64} with no AccountName");
            continue;
        };
        accounts.push((steam_id64, name.to_lowercase()));
    }
    Ok(accounts)
}

/// Every remembered Steam account whose refresh token can currently be decrypted.
/// Accounts that are logged out, have "remember me" off, or fail to decrypt are
/// skipped with a warning rather than failing the whole recovery — this is what
/// makes multi-account best-effort (some accounts simply can't do requests).
pub(crate) fn recover_all() -> Result<Vec<AuthContext>, GcError> {
    let steam =
        steamlocate::SteamDir::locate().map_err(|e| err(format!("Steam not found: {e}")))?;
    let steam_dir = steam.path();
    let local_vdf = local_vdf_path(steam_dir)?;
    let text = std::fs::read_to_string(&local_vdf)
        .map_err(|e| err(format!("cannot read local.vdf: {e}")))?;
    let vdf = keyvalues_parser::parse(&text)
        .map(Vdf::from)
        .map_err(|e| err(format!("cannot parse local.vdf: {e}")))?;

    let mut contexts = Vec::new();
    for (steam_id64, account_name) in all_account_ids(steam_dir)? {
        let recovered = connect_cache_blob(&vdf, &account_name)
            .and_then(|blob| decrypt_blob(&blob, &account_name))
            .and_then(|jwt| steam_id_from_jwt(&jwt).map(|sub| (jwt, sub)));
        match recovered {
            Ok((refresh_token, sub)) if sub == steam_id64 => contexts.push(AuthContext {
                account_name,
                steam_id64,
                refresh_token,
            }),
            // The ConnectCache lookup is keyed by account name, not steam_id64 - reject a
            // blob whose token identity doesn't match the loginusers.vdf entry it came from.
            Ok(_) => warn!(
                "skipping account {account_name}: token identity does not match loginusers.vdf entry"
            ),
            Err(e) => warn!("skipping account {account_name}: {e}"),
        }
    }

    if contexts.is_empty() {
        return Err(err("no decryptable Steam account found"));
    }
    Ok(contexts)
}

fn connect_cache_hex(vdf: &Vdf, account: &str) -> Result<Option<String>, GcError> {
    let cache = ["Software", "Valve", "Steam", "ConnectCache"]
        .into_iter()
        .try_fold(&vdf.value, child)
        .and_then(Value::get_obj)
        .ok_or_else(|| err("no ConnectCache in local.vdf (logged out or 'remember me' off)"))?;

    let prefix = format!("{:08x}", crc32fast::hash(account.as_bytes()));
    Ok(cache
        .iter()
        .find(|(subkey, _)| subkey.starts_with(&prefix))
        .and_then(|(_, values)| values.first())
        .and_then(Value::get_str)
        .map(str::to_owned))
}

fn connect_cache_blob(vdf: &Vdf, account: &str) -> Result<Vec<u8>, GcError> {
    let hex_value = connect_cache_hex(vdf, account)?
        .ok_or_else(|| err("no ConnectCache entry for this account"))?;
    hex::decode(hex_value).map_err(|e| err(format!("invalid ConnectCache hex: {e}")))
}

#[cfg(not(windows))]
fn decrypt_blob(blob: &[u8], account: &str) -> Result<String, GcError> {
    use aes::Aes256;
    use aes::cipher::generic_array::GenericArray;
    use aes::cipher::{BlockDecrypt, BlockDecryptMut, KeyInit, KeyIvInit, block_padding::Pkcs7};
    use sha2::{Digest, Sha256};

    if blob.len() < 32 {
        return Err(err("ConnectCache blob too short"));
    }
    let key = Sha256::digest(account.as_bytes());

    // The first block is the real IV, encrypted with AES-256-ECB.
    let cipher =
        Aes256::new_from_slice(key.as_slice()).map_err(|e| err(format!("aes key error: {e}")))?;
    let mut iv = GenericArray::clone_from_slice(&blob[0..16]);
    cipher.decrypt_block(&mut iv);

    let plaintext = cbc::Decryptor::<Aes256>::new_from_slices(key.as_slice(), iv.as_slice())
        .map_err(|e| err(format!("aes iv error: {e}")))?
        .decrypt_padded_vec_mut::<Pkcs7>(&blob[16..])
        .map_err(|e| err(format!("aes decrypt failed: {e}")))?;

    String::from_utf8(plaintext).map_err(|e| err(format!("token is not valid UTF-8: {e}")))
}

#[cfg(windows)]
fn decrypt_blob(blob: &[u8], account: &str) -> Result<String, GcError> {
    // DPAPI with the ASCII account name as the mandatory optional entropy.
    let plaintext =
        windows_dpapi::decrypt_data(blob, windows_dpapi::Scope::User, Some(account.as_bytes()))
            .map_err(|e| err(format!("DPAPI decrypt failed: {e}")))?;
    String::from_utf8(plaintext).map_err(|e| err(format!("token is not valid UTF-8: {e}")))
}

fn steam_id_from_jwt(jwt: &str) -> Result<u64, GcError> {
    let payload = jwt
        .split('.')
        .nth(1)
        .ok_or_else(|| err("token is not a JWT"))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|e| err(format!("cannot decode token payload: {e}")))?;
    let json: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| err(format!("cannot parse token payload: {e}")))?;

    if json.get("iss").and_then(serde_json::Value::as_str) != Some("steam") {
        return Err(err("token issuer is not steam"));
    }
    json.get("sub")
        .and_then(serde_json::Value::as_str)
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| err("token has no SteamID"))
}
