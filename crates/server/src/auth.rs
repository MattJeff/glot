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
    let password = b.password;
    let hash = tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut SaltRng))
            .map(|h| h.to_string())
            .map_err(|e| anyhow::anyhow!("argon2: {e}"))
    })
    .await
    .map_err(AppError::internal)??;
    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (id, email, name, password_hash) VALUES ($1, $2, $3, $4) RETURNING id, email, name",
    )
    .bind(Uuid::new_v4())
    .bind(&email)
    .bind(&name)
    .bind(hash)
    .fetch_one(&state.db)
    .await
    .map_err(|e| match AppError::from(e) {
        err if err.status == StatusCode::CONFLICT => AppError::new(StatusCode::CONFLICT, "email_taken", "email already registered"),
        err => err,
    })?;
    tracing::info!(user = %user.id, "signup");
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
    let password = b.password;
    let ok = tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash).is_ok_and(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
    })
    .await
    .map_err(AppError::internal)?;
    if !ok {
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

pub async fn me(AuthUser(user): AuthUser) -> Json<User> {
    Json(user)
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
