use crate::SecurityError;
use aether_core::OAuthProfile;
use oauth2::{
    basic::BasicClient, AuthUrl, AuthorizationCode, ClientId, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthPkceSession {
    pub authorization_url: String,
    pub csrf_state: String,
    pub pkce_verifier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokenResult {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in_secs: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct OAuthWorkflow {
    profile: OAuthProfile,
}

impl OAuthWorkflow {
    pub fn new(profile: OAuthProfile) -> Self {
        Self { profile }
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
