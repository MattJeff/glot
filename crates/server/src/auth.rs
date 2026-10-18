use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng},
};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng as SaltRng},
};
use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{ApiResult, AppError, AppJson, AppPath, AppState};

pub const SESSION_COOKIE: &str = "glot_session";
const SESSION_DAYS: i64 = 30;

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ApiToken {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

pub fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn random_token() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}

pub fn encrypt(secret: &[u8; 32], plaintext: &str) -> Vec<u8> {
    let cipher = Aes256Gcm::new(secret.into());
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    out.extend(
        cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .expect("aes-gcm encrypt"),
    );
    out
}

pub fn decrypt(secret: &[u8; 32], data: &[u8]) -> Option<String> {
    if data.len() < 12 {
        return None;
    }
    let cipher = Aes256Gcm::new(secret.into());
    let plain = cipher
        .decrypt(Nonce::from_slice(&data[..12]), &data[12..])
        .ok()?;
    String::from_utf8(plain).ok()
}

/// Authenticated user, from `Authorization: Bearer glot_...` or the `glot_session` cookie.
pub struct AuthUser(pub User);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        authenticate(&parts.headers, state)
            .await?
            .map(AuthUser)
            .ok_or_else(AppError::unauthorized)
    }
}

pub async fn authenticate(headers: &HeaderMap, state: &AppState) -> ApiResult<Option<User>> {
    if let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        let user = sqlx::query_as::<_, User>(
            "UPDATE api_tokens t SET last_used_at = now() FROM users u
             WHERE t.token_hash = $1 AND u.id = t.user_id RETURNING u.id, u.email, u.name",
        )
        .bind(sha256_hex(token.trim()))
        .fetch_optional(&state.db)
        .await?;
        return Ok(user);
    }
    let Some(token) = session_cookie(headers) else {
        return Ok(None);
    };
    Ok(sqlx::query_as::<_, User>(
        "SELECT u.id, u.email, u.name FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.expires_at > now()",
    )
    .bind(sha256_hex(&token))
    .fetch_optional(&state.db)
    .await?)
}

fn session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == SESSION_COOKIE)
        .map(|(_, v)| v.to_string())
}

fn cookie_header(state: &AppState, value: &str, max_age: i64) -> HeaderValue {
    let secure = if state.secure_cookies { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}{secure}"
    ))
    .expect("valid cookie")
}

async fn start_session(state: &AppState, user: User, status: StatusCode) -> ApiResult<Response> {
    let token = random_token();
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(sha256_hex(&token))
        .bind(user.id)
        .bind(Utc::now() + Duration::days(SESSION_DAYS))
        .execute(&state.db)
        .await?;
    let mut res = (status, Json(user)).into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        cookie_header(state, &token, SESSION_DAYS * 86400),
    );
    Ok(res)
}

#[derive(Deserialize)]
pub struct SignupBody {
    email: String,
    password: String,
    name: String,
    /// UI locale; emails are sent in it (unsupported => en).
    #[serde(default)]
    locale: Option<String>,
}

fn valid_email(e: &str) -> bool {
    let Some((local, domain)) = e.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && e.len() <= 254
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !e.chars().any(|c| c.is_whitespace())
        && !domain.contains('@')
}

pub async fn signup(
    State(state): State<AppState>,
    AppJson(b): AppJson<SignupBody>,
) -> ApiResult<Response> {
    let email = b.email.trim().to_lowercase();
    let name = b.name.trim().to_string();
    if !valid_email(&email) {
        return Err(AppError::bad_request("invalid email"));
    }
    if b.password.len() < 8 {
        return Err(AppError::bad_request(
            "password must be at least 8 characters",
        ));
    }
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::bad_request("name is required (max 100 chars)"));
    }
    let hash = hash_password(b.password).await?;
    let locale = b
        .locale
        .filter(|l| crate::email::supported_locale(l))
        .unwrap_or_else(|| "en".into());
    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (id, email, name, password_hash, locale) VALUES ($1, $2, $3, $4, $5) RETURNING id, email, name",
    )
    .bind(Uuid::new_v4())
    .bind(&email)
    .bind(&name)
    .bind(hash)
    .bind(&locale)
    .fetch_one(&state.db)
    .await
    .map_err(email_taken)?;
    tracing::info!(user = %user.id, "signup");
    if let Err(e) = send_verification(&state, &user, &locale, &user.email).await {
        tracing::error!(user = %user.id, "verification email: {}", e.message);
    }
    start_session(&state, user, StatusCode::CREATED).await
}

#[derive(Deserialize)]
pub struct LoginBody {
    email: String,
    password: String,
}

pub async fn login(
    State(state): State<AppState>,
    AppJson(b): AppJson<LoginBody>,
) -> ApiResult<Response> {
    let row: Option<(Uuid, String, String, String)> =
        sqlx::query_as("SELECT id, email, name, password_hash FROM users WHERE email = $1")
            .bind(b.email.trim().to_lowercase())
            .fetch_optional(&state.db)
            .await?;
    let invalid = || {
        AppError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "invalid email or password",
        )
    };
    let Some((id, email, name, hash)) = row else {
        return Err(invalid());
    };
    if !verify_password(hash, b.password).await? {
        return Err(invalid());
    }
    start_session(&state, User { id, email, name }, StatusCode::OK).await
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = session_cookie(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(sha256_hex(&token))
            .execute(&state.db)
            .await?;
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, cookie_header(&state, "", 0));
    Ok(res)
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Me {
    #[serde(flatten)]
    pub user: User,
    pub email_verified: bool,
    /// false when the server has no RESEND_API_KEY (no verification banner, no reset by email).
    pub email_enabled: bool,
    pub locale: String,
    pub notify_job_done: bool,
}

async fn load_me(state: &AppState, user: User) -> ApiResult<Me> {
    let (email_verified, locale, notify_job_done): (bool, String, bool) = sqlx::query_as(
        "SELECT email_verified_at IS NOT NULL, locale, notify_job_done FROM users WHERE id = $1",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    Ok(Me {
        user,
        email_verified,
        email_enabled: state.mailer.is_some(),
        locale,
        notify_job_done,
    })
}

pub async fn me(State(state): State<AppState>, AuthUser(user): AuthUser) -> ApiResult<Json<Me>> {
    Ok(Json(load_me(&state, user).await?))
}

#[derive(Deserialize)]
pub struct UpdateMeBody {
    locale: Option<String>,
    notify_job_done: Option<bool>,
}

pub async fn update_me(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppJson(b): AppJson<UpdateMeBody>,
) -> ApiResult<Json<Me>> {
    if let Some(l) = &b.locale
        && !crate::email::supported_locale(l)
    {
        return Err(AppError::bad_request(format!("unsupported locale '{l}'")));
    }
    sqlx::query(
        "UPDATE users SET locale = COALESCE($2, locale), notify_job_done = COALESCE($3, notify_job_done) WHERE id = $1",
    )
    .bind(user.id)
    .bind(&b.locale)
    .bind(b.notify_job_done)
    .execute(&state.db)
    .await?;
    Ok(Json(load_me(&state, user).await?))
}

pub async fn list_tokens(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> ApiResult<Json<Vec<ApiToken>>> {
    Ok(Json(
        sqlx::query_as("SELECT id, name, created_at, last_used_at FROM api_tokens WHERE user_id = $1 ORDER BY created_at DESC")
            .bind(user.id)
            .fetch_all(&state.db)
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct CreateTokenBody {
    name: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct CreatedToken {
    pub token: String,
    pub api_token: ApiToken,
}

pub async fn create_token(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppJson(b): AppJson<CreateTokenBody>,
) -> ApiResult<(StatusCode, Json<CreatedToken>)> {
    let name = b.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::bad_request("name is required (max 100 chars)"));
    }
    let token = format!("glot_{}", random_token());
    let api_token = sqlx::query_as(
        "INSERT INTO api_tokens (id, user_id, name, token_hash) VALUES ($1, $2, $3, $4)
         RETURNING id, name, created_at, last_used_at",
    )
    .bind(Uuid::new_v4())
    .bind(user.id)
    .bind(name)
    .bind(sha256_hex(&token))
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(CreatedToken { token, api_token })))
}

pub async fn delete_token(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(id): AppPath<Uuid>,
) -> ApiResult<StatusCode> {
    let res = sqlx::query("DELETE FROM api_tokens WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::not_found("token not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------- email verification, password reset, credential changes ----------

const VERIFY_HOURS: i64 = 48;
const RESET_MINUTES: i64 = 60;

pub fn email_routes() -> axum::Router<AppState> {
    use axum::routing::{post, put};
    axum::Router::new()
        .route("/api/auth/verify-email", post(verify_email))
        .route("/api/auth/verify-email/resend", post(resend_verification))
        .route("/api/auth/forgot-password", post(forgot_password))
        .route("/api/auth/reset-password", post(reset_password))
        .route("/api/me/password", put(change_password))
        .route("/api/me/email", put(change_email))
}

async fn hash_password(password: String) -> ApiResult<String> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut SaltRng))
            .map(|h| h.to_string())
            .map_err(|e| anyhow::anyhow!("argon2: {e}"))
    })
    .await
    .map_err(AppError::internal)?
    .map_err(AppError::from)
}

async fn verify_password(hash: String, password: String) -> ApiResult<bool> {
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash).is_ok_and(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
    })
    .await
    .map_err(AppError::internal)
}

fn email_taken(e: sqlx::Error) -> AppError {
    match AppError::from(e) {
        err if err.status == StatusCode::CONFLICT => taken(),
        err => err,
    }
}

fn taken() -> AppError {
    AppError::new(
        StatusCode::CONFLICT,
        "email_taken",
        "email already registered",
    )
}

fn invalid_token() -> AppError {
    AppError::new(
        StatusCode::BAD_REQUEST,
        "invalid_token",
        "this link is invalid, already used or expired",
    )
}

fn email_disabled() -> AppError {
    AppError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "email_disabled",
        "this server cannot send email (RESEND_API_KEY not set)",
    )
}

fn check_password(p: &str) -> ApiResult<()> {
    if p.len() < 8 {
        return Err(AppError::bad_request(
            "password must be at least 8 characters",
        ));
    }
    Ok(())
}

/// Single-use token; only its sha256 is stored.
async fn issue_token(
    state: &AppState,
    user: Uuid,
    purpose: &str,
    email: &str,
    ttl: Duration,
) -> ApiResult<String> {
    let token = random_token();
    sqlx::query(
        "INSERT INTO email_tokens (token_hash, user_id, purpose, email, expires_at) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(sha256_hex(&token))
    .bind(user)
    .bind(purpose)
    .bind(email)
    .bind(Utc::now() + ttl)
    .execute(&state.db)
    .await?;
    Ok(token)
}

/// Link confirming `email` (the account's address, or the new one during an email change).
async fn send_verification(
    state: &AppState,
    user: &User,
    locale: &str,
    email: &str,
) -> ApiResult<()> {
    let Some(m) = &state.mailer else {
        return Ok(());
    };
    let token = issue_token(
        state,
        user.id,
        "verify",
        email,
        Duration::hours(VERIFY_HOURS),
    )
    .await?;
    let url = m.link(locale, &format!("/verify-email?token={token}"));
    let to = crate::email::To {
        email,
        name: &user.name,
        locale,
    };
    let idem = format!("verify:{}", sha256_hex(&token));
    crate::email::send(state, to, "verify", &[], &url, &idem).await;
    Ok(())
}

/// "Your password/email changed" notice, with a reset link in case it wasn't the owner.
async fn send_notice(
    state: &AppState,
    to: crate::email::To<'_>,
    template: &str,
    vars: &[(&str, &str)],
    idem: &str,
) {
    if let Some(m) = &state.mailer {
        let url = m.link(to.locale, "/forgot-password");
        crate::email::send(state, to, template, vars, &url, idem).await;
    }
}

#[derive(Deserialize)]
pub struct TokenBody {
    token: String,
}

/// Confirms an address (no session needed: the token is the proof). For an email change, also switches
/// `users.email` and notifies the previous address.
pub async fn verify_email(
    State(state): State<AppState>,
    AppJson(b): AppJson<TokenBody>,
) -> ApiResult<StatusCode> {
    let hash = sha256_hex(b.token.trim());
    let mut tx = state.db.begin().await?;
    let (user_id, email): (Uuid, String) = sqlx::query_as(
        "UPDATE email_tokens SET used_at = now()
         WHERE token_hash = $1 AND purpose = 'verify' AND used_at IS NULL AND expires_at > now()
         RETURNING user_id, email",
    )
    .bind(&hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(invalid_token)?;
    let (old, name, locale): (String, String, String) = sqlx::query_as(
        "UPDATE users u SET email = $2, email_verified_at = now() FROM users o
         WHERE u.id = $1 AND o.id = $1 RETURNING o.email, u.name, u.locale",
    )
    .bind(user_id)
    .bind(&email)
    .fetch_one(&mut *tx)
    .await
    .map_err(email_taken)?;
    tx.commit().await?;
    if old != email {
        tracing::info!(user = %user_id, "email changed");
        let to = crate::email::To {
            email: &old,
            name: &name,
            locale: &locale,
        };
        send_notice(
            &state,
            to,
            "email_changed",
            &[("new_email", &email)],
            &format!("email_changed:{hash}"),
        )
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn resend_verification(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> ApiResult<StatusCode> {
    if state.mailer.is_none() {
        return Err(email_disabled());
    }
    let me = load_me(&state, user).await?;
    if !me.email_verified {
        send_verification(&state, &me.user, &me.locale, &me.user.email).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ForgotBody {
    email: String,
}

/// Always 204 (no account enumeration). Per-recipient hourly budget applies (email::send).
pub async fn forgot_password(
    State(state): State<AppState>,
    AppJson(b): AppJson<ForgotBody>,
) -> ApiResult<StatusCode> {
    let Some(m) = &state.mailer else {
        return Err(email_disabled());
    };
    let row: Option<(Uuid, String, String, String)> =
        sqlx::query_as("SELECT id, email, name, locale FROM users WHERE email = $1")
            .bind(b.email.trim().to_lowercase())
            .fetch_optional(&state.db)
            .await?;
    if let Some((id, email, name, locale)) = row {
        let token = issue_token(
            &state,
            id,
            "reset",
            &email,
            Duration::minutes(RESET_MINUTES),
        )
        .await?;
        let url = m.link(&locale, &format!("/reset-password?token={token}"));
        let to = crate::email::To {
            email: &email,
            name: &name,
            locale: &locale,
        };
        let idem = format!("reset:{}", sha256_hex(&token));
        crate::email::send(&state, to, "reset", &[], &url, &idem).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ResetBody {
    token: String,
    password: String,
}

/// Token: 1 h, single use, only valid while the account still has the address it was sent to.
/// Success signs the user out everywhere and burns the other reset links.
pub async fn reset_password(
    State(state): State<AppState>,
    AppJson(b): AppJson<ResetBody>,
) -> ApiResult<StatusCode> {
    check_password(&b.password)?;
    let new_hash = hash_password(b.password).await?;
    let hash = sha256_hex(b.token.trim());
    let mut tx = state.db.begin().await?;
    let (id, email, name, locale): (Uuid, String, String, String) = sqlx::query_as(
        "UPDATE email_tokens t SET used_at = now() FROM users u
         WHERE t.token_hash = $1 AND t.purpose = 'reset' AND t.used_at IS NULL AND t.expires_at > now()
           AND u.id = t.user_id AND u.email = t.email
         RETURNING u.id, u.email, u.name, u.locale",
    )
    .bind(&hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(invalid_token)?;
    // Reading the inbox proves the address too.
    sqlx::query("UPDATE users SET password_hash = $2, email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1")
        .bind(id)
        .bind(new_hash)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE email_tokens SET used_at = now() WHERE user_id = $1 AND purpose = 'reset' AND used_at IS NULL")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(user = %id, "password reset");
    let to = crate::email::To {
        email: &email,
        name: &name,
        locale: &locale,
    };
    send_notice(
        &state,
        to,
        "password_changed",
        &[],
        &format!("password_changed:{hash}"),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ChangePasswordBody {
    current_password: String,
    password: String,
}

async fn check_current_password(
    state: &AppState,
    user: Uuid,
    password: String,
) -> ApiResult<String> {
    let (hash, locale): (String, String) =
        sqlx::query_as("SELECT password_hash, locale FROM users WHERE id = $1")
            .bind(user)
            .fetch_one(&state.db)
            .await?;
    if !verify_password(hash, password).await? {
        return Err(AppError::new(
            StatusCode::FORBIDDEN,
            "invalid_password",
            "current password is incorrect",
        ));
    }
    Ok(locale)
}

/// Keeps the calling session, signs out the others.
pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    AuthUser(user): AuthUser,
    AppJson(b): AppJson<ChangePasswordBody>,
) -> ApiResult<StatusCode> {
    check_password(&b.password)?;
    let locale = check_current_password(&state, user.id, b.current_password).await?;
    let new_hash = hash_password(b.password).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
        .bind(user.id)
        .bind(new_hash)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND token_hash IS DISTINCT FROM $2")
        .bind(user.id)
        .bind(session_cookie(&headers).map(|t| sha256_hex(&t)))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let to = crate::email::To {
        email: &user.email,
        name: &user.name,
        locale: &locale,
    };
    send_notice(
        &state,
        to,
        "password_changed",
        &[],
        &format!("password_changed:{}", Uuid::new_v4()),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ChangeEmailBody {
    email: String,
    password: String,
}

#[derive(Serialize)]
pub struct ChangeEmailResult {
    /// true: a confirmation link was sent to the new address, the change applies when it is opened.
    pending: bool,
}

/// With email enabled the new address must be confirmed first (then the old one is notified);
/// without, the change is immediate.
pub async fn change_email(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppJson(b): AppJson<ChangeEmailBody>,
) -> ApiResult<Json<ChangeEmailResult>> {
    let email = b.email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(AppError::bad_request("invalid email"));
    }
    let locale = check_current_password(&state, user.id, b.password).await?;
    if email == user.email {
        return Err(AppError::bad_request("this is already your email"));
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE email = $1)")
        .bind(&email)
        .fetch_one(&state.db)
        .await?;
    if exists {
        return Err(taken());
    }
    if state.mailer.is_none() {
        sqlx::query("UPDATE users SET email = $2, email_verified_at = NULL WHERE id = $1")
            .bind(user.id)
            .bind(&email)
            .execute(&state.db)
            .await
            .map_err(email_taken)?;
        return Ok(Json(ChangeEmailResult { pending: false }));
    }
    send_verification(&state, &user, &locale, &email).await?;
    Ok(Json(ChangeEmailResult { pending: true }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn crypto_roundtrip_and_email() {
        let k = [7u8; 32];
        let enc = super::encrypt(&k, "sk-secret");
        assert_eq!(super::decrypt(&k, &enc).as_deref(), Some("sk-secret"));
        assert!(super::decrypt(&[8u8; 32], &enc).is_none());
        assert!(super::valid_email("a@b.co"));
        assert!(!super::valid_email("a@b"));
        assert!(!super::valid_email("a b@c.de"));
    }
}
