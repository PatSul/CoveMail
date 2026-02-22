use crate::SecurityError;
use cove_core::OAuthProfile;
use oauth2::{
    basic::BasicClient, AuthUrl, AuthorizationCode, ClientId, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthPkceSession {
    pub authorization_url: String,
    pub csrf_state: String,
    pub pkce_verifier: String,
}

impl std::fmt::Debug for OAuthPkceSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthPkceSession")
            .field("authorization_url", &self.authorization_url)
            .field("csrf_state", &"[REDACTED]")
            .field("pkce_verifier", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthTokenResult {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in_secs: Option<u64>,
}

impl std::fmt::Debug for OAuthTokenResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthTokenResult")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "[REDACTED]"))
            .field("expires_in_secs", &self.expires_in_secs)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct OAuthWorkflow {
    profile: OAuthProfile,
}

/// Known overly-broad scopes that should be rejected.
const DANGEROUS_SCOPES: &[&str] = &["*", "admin", "root", "full_access"];

impl OAuthWorkflow {
    /// Create a new OAuth workflow, validating the profile for security issues.
    pub fn new(profile: OAuthProfile) -> Result<Self, SecurityError> {
        Self::validate_profile(&profile)?;
        Ok(Self { profile })
    }

    /// Validate an OAuth profile for security before use.
    fn validate_profile(profile: &OAuthProfile) -> Result<(), SecurityError> {
        // Enforce HTTPS on auth and token endpoints.
        if profile.auth_url.scheme() != "https" {
            return Err(SecurityError::OAuth(
                "Authorization URL must use HTTPS".to_string(),
            ));
        }
        if profile.token_url.scheme() != "https" {
            return Err(SecurityError::OAuth(
                "Token URL must use HTTPS".to_string(),
            ));
        }

        // Redirect URL must be localhost for desktop apps.
        let redirect_host = profile.redirect_url.host_str().unwrap_or("");
        let is_localhost = redirect_host == "127.0.0.1"
            || redirect_host == "localhost"
            || redirect_host == "[::1]";
        if !is_localhost {
            return Err(SecurityError::OAuth(
                "Redirect URL must point to localhost (127.0.0.1 or localhost) for desktop apps"
                    .to_string(),
            ));
        }

        // Client ID basic validation.
        let client_id = profile.client_id.trim();
        if client_id.is_empty() {
            return Err(SecurityError::OAuth("Client ID is required".to_string()));
        }
        if client_id.len() > 512 {
            return Err(SecurityError::OAuth(
                "Client ID appears invalid (too long)".to_string(),
            ));
        }

        // Reject dangerous scopes.
        for scope in &profile.scopes {
            let lower = scope.to_lowercase();
            if DANGEROUS_SCOPES.iter().any(|&d| lower == d) {
                return Err(SecurityError::OAuth(format!(
                    "Scope '{scope}' is too broad and not allowed"
                )));
            }
        }

        // Auth and token endpoints must have a valid host.
        if profile.auth_url.host_str().is_none() {
            return Err(SecurityError::OAuth(
                "Authorization URL must have a valid host".to_string(),
            ));
        }
        if profile.token_url.host_str().is_none() {
            return Err(SecurityError::OAuth(
                "Token URL must have a valid host".to_string(),
            ));
        }

        Ok(())
    }

    pub fn begin_pkce_session(&self) -> Result<OAuthPkceSession, SecurityError> {
        let client = BasicClient::new(ClientId::new(self.profile.client_id.clone()))
            .set_auth_uri(AuthUrl::new(self.profile.auth_url.as_str().to_string())?)
            .set_token_uri(TokenUrl::new(self.profile.token_url.as_str().to_string())?)
            .set_redirect_uri(RedirectUrl::new(
                self.profile.redirect_url.as_str().to_string(),
            )?);

        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let mut auth_request = client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(pkce_challenge);

        for scope in &self.profile.scopes {
            auth_request = auth_request.add_scope(Scope::new(scope.to_string()));
        }

        let (auth_url, csrf_state) = auth_request.url();

        Ok(OAuthPkceSession {
            authorization_url: auth_url.to_string(),
            csrf_state: csrf_state.secret().to_string(),
            pkce_verifier: pkce_verifier.secret().to_string(),
        })
    }

    pub async fn exchange_code(
        &self,
        code: &str,
        pkce_verifier: &str,
    ) -> Result<OAuthTokenResult, SecurityError> {
        let client = BasicClient::new(ClientId::new(self.profile.client_id.clone()))
            .set_auth_uri(AuthUrl::new(self.profile.auth_url.as_str().to_string())?)
            .set_token_uri(TokenUrl::new(self.profile.token_url.as_str().to_string())?)
            .set_redirect_uri(RedirectUrl::new(
                self.profile.redirect_url.as_str().to_string(),
            )?);

        let http_client = reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        let token = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_string()))
            .request_async(&http_client)
            .await
            .map_err(|err| SecurityError::OAuth(err.to_string()))?;

        Ok(OAuthTokenResult {
            access_token: token.access_token().secret().to_string(),
            refresh_token: token
                .refresh_token()
                .map(|token| token.secret().to_string()),
            expires_in_secs: token.expires_in().map(|duration| duration.as_secs()),
        })
    }
}
