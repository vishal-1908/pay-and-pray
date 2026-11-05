//! Credentials wrapper around the OS keychain.
//!
//! All values live under the service name `pay-and-pray`. Per-account keys:
//!
//! - `claude.session`   — the `sessionKey` cookie value
//! - `claude.orgId`     — cached Claude organization UUID
//! - `claude.oauth`     — marker: the user opted in to reading Claude Code's own
//!   OAuth login (we never copy that token; see [`Credentials::claude_code_token`])
//! - `copilot.token`    — GitHub Bearer token (any classic/OAuth token)
//!
//! Multiple GitHub Copilot accounts are supported. Each connected account is
//! keyed by its GitHub user id:
//!
//! - `copilot.accounts`     — JSON registry: `[{ id, login, label }, …]`
//! - `copilot.token.<id>`   — that account's GitHub Bearer token
//!
//! `copilot.token` (no id) is the pre-multi-account single-token key; it's read
//! only for one-time migration into the registry. `copilot.username` /
//! `copilot.plan` are older legacy keys from the original billing API; all three
//! stay in the sign-out cleanup so upgraded installs are wiped clean.

use keyring::Entry;
use serde::{Deserialize, Serialize};
use tracing::warn;

const SERVICE: &str = "pay-and-pray";
const CLAUDE_CODE_SERVICE: &str = "Claude Code-credentials";
const COPILOT_ACCOUNTS_KEY: &str = "copilot.accounts";

/// A connected GitHub Copilot account. `id` is the GitHub user id (stable across
/// login renames); `label` is the user-facing name shown on the card, defaulting
/// to the login but renameable (e.g. to the org it belongs to).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopilotAccount {
    pub id: String,
    pub login: String,
    pub label: String,
}

fn copilot_token_key(id: &str) -> String {
    format!("copilot.token.{id}")
}

pub struct Credentials;

impl Credentials {
    pub fn new() -> Self {
        Self
    }

    pub fn claude_session(&self) -> Option<String> {
        get("claude.session")
    }

    /// Whether the user chose "Use Claude Code login". Gates every read of Claude
    /// Code's keychain item so we never trigger its access prompt uninvited.
    pub fn claude_oauth_enabled(&self) -> bool {
        get("claude.oauth").is_some()
    }

    /// Claude Code's OAuth access token, read live from its own keychain item
    /// (macOS only). Never copied into our keychain — Claude Code refreshes it.
    /// `None` when absent, unreadable or expired.
    pub fn claude_code_token(&self) -> Option<String> {
        let account = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .ok()?;
        let raw = match Entry::new(CLAUDE_CODE_SERVICE, &account).and_then(|e| e.get_password()) {
            Ok(v) => v,
            Err(keyring::Error::NoEntry) => return None,
            Err(e) => {
                warn!(error = ?e, "claude code keychain read failed");
                return None;
            }
        };
        parse_claude_code_token(&raw, chrono::Utc::now().timestamp_millis())
    }

    pub fn claude_org_id(&self) -> Option<String> {
        get("claude.orgId")
    }

    /// The legacy single-token value (pre-multi-account). Read only for migration
    /// and as a fallback source for diagnostics.
    pub fn copilot_token(&self) -> Option<String> {
        get("copilot.token")
    }

    /// The connected Copilot accounts, newest last. Empty when none are connected
    /// (or only a not-yet-migrated legacy token exists).
    pub fn copilot_accounts(&self) -> Vec<CopilotAccount> {
        get(COPILOT_ACCOUNTS_KEY)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn copilot_token_for(&self, id: &str) -> Option<String> {
        get(&copilot_token_key(id))
    }

    fn write_copilot_accounts(&self, accounts: &[CopilotAccount]) -> anyhow::Result<()> {
        self.set(COPILOT_ACCOUNTS_KEY, &serde_json::to_string(accounts)?)
    }

    /// Add or update a Copilot account and store its token. Re-adding an existing
    /// id refreshes the token and login but keeps any user-set label.
    pub fn add_copilot_account(&self, account: CopilotAccount, token: &str) -> anyhow::Result<()> {
        self.set(&copilot_token_key(&account.id), token)?;
        let mut accounts = self.copilot_accounts();
        upsert_account(&mut accounts, account);
        self.write_copilot_accounts(&accounts)
    }

    /// Remove a Copilot account and delete its token.
    pub fn remove_copilot_account(&self, id: &str) -> anyhow::Result<()> {
        self.delete(&copilot_token_key(id))?;
        let mut accounts = self.copilot_accounts();
        accounts.retain(|a| a.id != id);
        self.write_copilot_accounts(&accounts)
    }

    /// Rename a Copilot account's display label.
    pub fn set_copilot_label(&self, id: &str, label: &str) -> anyhow::Result<()> {
        let mut accounts = self.copilot_accounts();
        if let Some(a) = accounts.iter_mut().find(|a| a.id == id) {
            a.label = label.to_string();
        }
        self.write_copilot_accounts(&accounts)
    }

    /// Per-account token keys currently in the registry — for sign-out cleanup.
    pub fn copilot_account_token_keys(&self) -> Vec<String> {
        self.copilot_accounts()
            .iter()
            .map(|a| copilot_token_key(&a.id))
            .collect()
    }

    pub fn set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let entry = Entry::new(SERVICE, key)?;
        entry.set_password(value)?;
        Ok(())
    }

    pub fn delete(&self, key: &str) -> anyhow::Result<()> {
        let entry = Entry::new(SERVICE, key)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Insert `account`, or update the matching id in place. A re-add refreshes the
/// login and only fills the label when the existing one is blank, so a user's
/// rename survives re-authentication.
fn upsert_account(accounts: &mut Vec<CopilotAccount>, account: CopilotAccount) {
    match accounts.iter_mut().find(|a| a.id == account.id) {
        Some(existing) => {
            existing.login = account.login;
            if existing.label.trim().is_empty() {
                existing.label = account.label;
            }
        }
        None => accounts.push(account),
    }
}

/// Extract `claudeAiOauth.accessToken` from Claude Code's credentials JSON,
/// rejecting an empty or already-expired token (`expiresAt` is epoch millis).
fn parse_claude_code_token(raw: &str, now_ms: i64) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let oauth = v.get("claudeAiOauth")?;
    let token = oauth.get("accessToken")?.as_str()?.trim();
    if token.is_empty() {
        return None;
    }
    if let Some(exp) = oauth.get("expiresAt").and_then(|e| e.as_i64()) {
        if exp <= now_ms {
            return None;
        }
    }
    Some(token.to_string())
}

fn get(key: &str) -> Option<String> {
    match Entry::new(SERVICE, key).and_then(|e| e.get_password()) {
        Ok(v) => Some(v),
        Err(keyring::Error::NoEntry) => None,
        Err(e) => {
            warn!(key, error = ?e, "keychain read failed");
            None
        }
    }
}

/// The *static* keychain keys owned by a service, used to clear credentials on
/// sign-out. For Copilot the per-account `copilot.token.<id>` keys are dynamic —
/// enumerate them via [`Credentials::copilot_account_token_keys`] in addition to
/// these. Returns an error for an unknown service name. The legacy
/// `copilot.token` / `copilot.username` / `copilot.plan` keys are listed so
/// sign-out wipes them from pre-migration installs.
pub fn service_keys(service: &str) -> Result<&'static [&'static str], String> {
    match service {
        "claude" => Ok(&["claude.session", "claude.orgId", "claude.oauth"]),
        "copilot" => Ok(&[
            "copilot.accounts",
            "copilot.token",
            "copilot.username",
            "copilot.plan",
        ]),
        other => Err(format!("unknown service: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_keys_cover_each_prefix() {
        let claude = service_keys("claude").unwrap();
        assert_eq!(claude, &["claude.session", "claude.orgId", "claude.oauth"]);
        assert!(claude.iter().all(|k| k.starts_with("claude.")));

        let copilot = service_keys("copilot").unwrap();
        assert_eq!(
            copilot,
            &[
                "copilot.accounts",
                "copilot.token",
                "copilot.username",
                "copilot.plan"
            ]
        );
        assert!(copilot.iter().all(|k| k.starts_with("copilot.")));
    }

    #[test]
    fn service_keys_rejects_unknown_service() {
        let err = service_keys("cursor").unwrap_err();
        assert!(err.contains("cursor"), "error should name the service");
    }

    #[test]
    fn claude_code_token_parses_and_checks_expiry() {
        let json = r#"{"claudeAiOauth":{"accessToken":"tok","expiresAt":2000}}"#;
        assert_eq!(parse_claude_code_token(json, 1000).as_deref(), Some("tok"));
        assert_eq!(parse_claude_code_token(json, 2000), None);
        assert_eq!(
            parse_claude_code_token(r#"{"claudeAiOauth":{"accessToken":"tok"}}"#, 1000).as_deref(),
            Some("tok")
        );
        assert_eq!(
            parse_claude_code_token(r#"{"claudeAiOauth":{"accessToken":""}}"#, 0),
            None
        );
        assert_eq!(parse_claude_code_token("not json", 0), None);
    }

    fn acct(id: &str, login: &str, label: &str) -> CopilotAccount {
        CopilotAccount {
            id: id.into(),
            login: login.into(),
            label: label.into(),
        }
    }

    #[test]
    fn upsert_appends_new_accounts() {
        let mut accounts = vec![acct("1", "alice", "alice")];
        upsert_account(&mut accounts, acct("2", "bob", "bob"));
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[1].id, "2");
    }

    #[test]
    fn upsert_refreshes_login_but_keeps_user_label() {
        // alice renamed her account to "Acme Corp"; a re-auth must not clobber it,
        // but should pick up a changed GitHub login.
        let mut accounts = vec![acct("1", "alice", "Acme Corp")];
        upsert_account(&mut accounts, acct("1", "alice-acme", "alice-acme"));
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].login, "alice-acme");
        assert_eq!(accounts[0].label, "Acme Corp");
    }

    #[test]
    fn upsert_fills_blank_label_on_reauth() {
        let mut accounts = vec![acct("1", "alice", "   ")];
        upsert_account(&mut accounts, acct("1", "alice", "alice"));
        assert_eq!(accounts[0].label, "alice");
    }
}
