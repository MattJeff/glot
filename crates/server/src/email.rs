//! Transactional email through the Resend HTTP API (no SMTP).
//! Postgres outbox (`email_outbox`): rendered at enqueue, delivered by a worker with retries/backoff,
//! deduped by `idem_key` (also sent as Resend's `Idempotency-Key`). Off when `RESEND_API_KEY` is unset.
//! Copy lives in `crates/server/emails/{locale}.json`, translated by glot (`glot.toml` next to it).
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::sync::Notify;
use uuid::Uuid;

use crate::AppState;

static CATALOGS: LazyLock<HashMap<&'static str, Value>> = LazyLock::new(|| {
    [
        ("en", include_str!("../emails/en.json")),
        ("fr", include_str!("../emails/fr.json")),
        ("es", include_str!("../emails/es.json")),
        ("de", include_str!("../emails/de.json")),
        ("ja", include_str!("../emails/ja.json")),
        ("ar", include_str!("../emails/ar.json")),
    ]
    .into_iter()
    .map(|(l, s)| (l, serde_json::from_str(s).expect("email catalog json")))
    .collect()
});

const MAX_ATTEMPTS: i32 = 6;

#[derive(Clone)]
pub struct Mailer {
    http: reqwest::Client,
    api_key: String,
    /// `https://api.resend.com` (tests point it at a fake server).
    pub api_url: String,
    pub from: String,
    pub reply_to: Option<String>,
    /// Dashboard origin used in links (GLOT_WEB_ORIGIN).
    pub app_url: String,
    /// Max emails per recipient per hour (all templates).
    pub rate_per_hour: i64,
    /// First retry delay, doubled each attempt.
    pub retry_base: Duration,
    wakeup: Arc<Notify>,
}

impl Mailer {
    pub fn new(api_key: &str, api_url: &str, from: &str, app_url: &str) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("reqwest client"),
            api_key: api_key.into(),
            api_url: api_url.trim_end_matches('/').into(),
            from: from.into(),
            reply_to: None,
            app_url: app_url.trim_end_matches('/').into(),
            rate_per_hour: 10,
            retry_base: Duration::from_secs(30),
            wakeup: Arc::new(Notify::new()),
        }
    }

    /// `None` (email disabled, e.g. local dev) when RESEND_API_KEY is unset.
    pub fn from_env(app_url: &str) -> Option<Self> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let Some(key) = var("RESEND_API_KEY") else {
            tracing::info!("RESEND_API_KEY not set: transactional email disabled");
            return None;
        };
        let mut m = Self::new(
            key.trim(),
            &var("RESEND_API_URL").unwrap_or_else(|| "https://api.resend.com".into()),
            &var("RESEND_FROM").unwrap_or_else(|| "glot <onboarding@resend.dev>".into()),
            app_url,
        );
        m.reply_to = var("RESEND_REPLY_TO");
        if let Some(n) = var("GLOT_EMAIL_RATE_PER_HOUR").and_then(|v| v.parse().ok()) {
            m.rate_per_hour = n;
        }
        tracing::info!(from = %m.from, "transactional email enabled (Resend)");
        Some(m)
    }

    /// Dashboard link, locale-prefixed like next-intl's `as-needed` routing.
    pub fn link(&self, locale: &str, path: &str) -> String {
        let prefix = if locale == "en" || !CATALOGS.contains_key(locale) {
            String::new()
        } else {
            format!("/{locale}")
        };
        format!("{}{prefix}{path}", self.app_url)
    }
}

pub fn supported_locale(l: &str) -> bool {
    CATALOGS.contains_key(l)
}

// ---------- rendering ----------

fn t(locale: &str, key: &str) -> String {
    let get = |l: &str| {
        CATALOGS
            .get(l)?
            .pointer(&format!("/emails/{}", key.replace('.', "/")))?
            .as_str()
            .map(str::to_string)
    };
    get(locale)
        .or_else(|| get("en"))
        .unwrap_or_else(|| key.into())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `{var}` substitution (the catalog uses no plural/select: keep it that way).
fn fill(s: &str, vars: &[(&str, &str)], html: bool) -> String {
    let mut out = if html { esc(s) } else { s.to_string() };
    for (k, v) in vars {
        let v = if html { esc(v) } else { v.to_string() };
        out = out.replace(&format!("{{{k}}}"), &v);
    }
    out
}

pub struct Rendered {
    pub subject: String,
    pub html: String,
    pub text: String,
}

/// Account emails (`optional = false`) say they can't be unsubscribed; job notices point to settings.
pub fn render(
    locale: &str,
    template: &str,
    name: &str,
    email: &str,
    vars: &[(&str, &str)],
    cta_url: &str,
) -> Rendered {
    let locale = if supported_locale(locale) {
        locale
    } else {
        "en"
    };
    let mut all: Vec<(&str, &str)> = vec![("name", name), ("email", email)];
    all.extend_from_slice(vars);
    let tr = |k: &str, html: bool| fill(&t(locale, k), &all, html);
    let subject = tr(&format!("{template}.subject"), false);
    let footer_key = if template.starts_with("job_") {
        "common.footer_optional"
    } else {
        "common.footer"
    };
    let ignore = matches!(template, "verify" | "reset");
    let dir = if locale == "ar" { "rtl" } else { "ltr" };
    let p = "margin:0 0 16px;font-size:15px;line-height:24px";
    let url = esc(cta_url);
    let html = format!(
        r#"<!doctype html>
<html lang="{locale}" dir="{dir}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="light"><title>{subject_h}</title></head>
<body style="margin:0;padding:0;background:#f4f4f5;color:#18181b;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif">
<div style="display:none;max-height:0;overflow:hidden">{body}</div>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#f4f4f5"><tr><td align="center" style="padding:32px 12px">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="max-width:520px;background:#ffffff;border:1px solid #e4e4e7;border-radius:12px"><tr><td style="padding:32px 28px">
<p style="margin:0 0 24px;font-size:17px;font-weight:700;letter-spacing:-0.02em">glot</p>
<h1 style="margin:0 0 16px;font-size:20px;line-height:28px;font-weight:600">{subject_h}</h1>
<p style="{p}">{greeting}</p>
<p style="{p}">{body}</p>
<table role="presentation" cellpadding="0" cellspacing="0" style="margin:24px 0"><tr><td style="border-radius:8px;background:#18181b"><a href="{url}" style="display:inline-block;padding:12px 20px;font-size:15px;font-weight:600;color:#ffffff;text-decoration:none">{cta}</a></td></tr></table>
<p style="margin:0 0 16px;font-size:13px;line-height:20px;color:#71717a">{fallback}<br><a href="{url}" style="color:#52525b;word-break:break-all">{url}</a></p>
{ignore_h}</td></tr></table>
<p style="max-width:520px;margin:16px auto 0;font-size:12px;line-height:18px;color:#71717a">{footer}</p>
</td></tr></table></body></html>"#,
        subject_h = esc(&subject),
        greeting = tr("common.greeting", true),
        body = tr(&format!("{template}.body"), true),
        cta = tr(&format!("{template}.cta"), true),
        fallback = tr("common.fallback", true),
        ignore_h = if ignore {
            format!(
                r#"<p style="margin:0;font-size:13px;line-height:20px;color:#71717a">{}</p>"#,
                tr("common.ignore", true)
            )
        } else {
            String::new()
        },
        footer = tr(footer_key, true),
    );
    let text = format!(
        "{subject}\n\n{}\n\n{}\n\n{}: {cta_url}\n\n{}--\n{}\n",
        tr("common.greeting", false),
        tr(&format!("{template}.body"), false),
        tr(&format!("{template}.cta"), false),
        if ignore {
            format!("{}\n\n", tr("common.ignore", false))
        } else {
            String::new()
        },
        tr(footer_key, false),
    );
    Rendered {
        subject,
        html,
        text,
    }
}

// ---------- outbox ----------

pub struct To<'a> {
    pub email: &'a str,
    pub name: &'a str,
    pub locale: &'a str,
}

/// Renders and enqueues. Best effort: never fails the caller (logged). Returns whether a row was queued
/// (false = email disabled, duplicate `idem_key`, or recipient over its hourly budget).
pub async fn send(
    state: &AppState,
    to: To<'_>,
    template: &str,
    vars: &[(&str, &str)],
    cta_url: &str,
    idem_key: &str,
) -> bool {
    let Some(m) = &state.mailer else {
        return false;
    };
    let r = render(to.locale, template, to.name, to.email, vars, cta_url);
    // ponytail: count-then-insert in one statement; two concurrent enqueues may both pass the limit by one.
    let res = sqlx::query(
        "INSERT INTO email_outbox (id, idem_key, to_email, template, subject, html, text_body)
         SELECT $1, $2, $3, $4, $5, $6, $7
         WHERE (SELECT count(*) FROM email_outbox WHERE to_email = $3 AND created_at > now() - interval '1 hour') < $8
         ON CONFLICT (idem_key) DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(idem_key)
    .bind(to.email)
    .bind(template)
    .bind(&r.subject)
    .bind(&r.html)
    .bind(&r.text)
    .bind(m.rate_per_hour)
    .execute(&state.db)
    .await;
    match res {
        Ok(r) if r.rows_affected() == 1 => {
            m.wakeup.notify_one();
            true
        }
        Ok(_) => {
            tracing::info!(
                template,
                "email not queued (duplicate or recipient rate limit)"
            );
            false
        }
        Err(e) => {
            tracing::error!(template, "email enqueue failed: {e}");
            false
        }
    }
}

/// Opt-in notice to the user who started a job (dashboard/API `POST …/jobs`).
#[allow(clippy::too_many_arguments)]
pub async fn job_finished(
    state: &AppState,
    job_id: Uuid,
    ok: bool,
    done: i32,
    total: i32,
    cost_usd: f64,
    error: Option<&str>,
) {
    if state.mailer.is_none() {
        return;
    }
    let row: Option<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT u.email, u.name, u.locale, p.name, p.slug FROM jobs j
         JOIN users u ON u.id = j.created_by JOIN projects p ON p.id = j.project_id
         WHERE j.id = $1 AND u.notify_job_done",
    )
    .bind(job_id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or_else(|e| {
        tracing::error!("job email lookup failed: {e}");
        None
    });
    let Some((email, name, locale, project, slug)) = row else {
        return;
    };
    let url = state
        .mailer
        .as_ref()
        .map(|m| m.link(&locale, &format!("/app/{slug}/jobs")))
        .unwrap_or_default();
    let (done, total, cost) = (
        done.to_string(),
        total.to_string(),
        format!("{cost_usd:.4}"),
    );
    let error: String = error.unwrap_or("").chars().take(300).collect();
    send(
        state,
        To {
            email: &email,
            name: &name,
            locale: &locale,
        },
        if ok { "job_done" } else { "job_failed" },
        &[
            ("project", &project),
            ("done", &done),
            ("total", &total),
            ("cost", &cost),
            ("error", &error),
        ],
        &url,
        &format!("job:{job_id}"),
    )
    .await;
}

// ---------- delivery ----------

pub fn spawn_worker(state: AppState) -> Option<tokio::task::JoinHandle<()>> {
    let m = state.mailer.clone()?;
    Some(tokio::spawn(async move {
        loop {
            match deliver_one(&state.db, &m).await {
                Ok(true) => {}
                Ok(false) => {
                    let _ = tokio::time::timeout(Duration::from_secs(5), m.wakeup.notified()).await;
                }
                Err(e) => {
                    tracing::error!("email worker: {e}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }))
}

enum Outcome {
    Sent(String),
    Retry(String),
    Fatal(String),
}

/// Claims one due email (`SKIP LOCKED`, safe across replicas) and sends it. The claim pushes
/// `next_attempt_at` 5 min ahead as a lease: a crash mid-send retries later, Idempotency-Key prevents a double send.
async fn deliver_one(db: &sqlx::PgPool, m: &Mailer) -> sqlx::Result<bool> {
    let row: Option<(Uuid, String, String, Option<String>, Option<String>, i32)> = sqlx::query_as(
        "UPDATE email_outbox SET attempts = attempts + 1, next_attempt_at = now() + interval '5 minutes'
         WHERE id = (SELECT id FROM email_outbox WHERE status = 'queued' AND next_attempt_at <= now()
                     ORDER BY next_attempt_at FOR UPDATE SKIP LOCKED LIMIT 1)
         RETURNING id, to_email, subject, html, text_body, attempts",
    )
    .fetch_optional(db)
    .await?;
    let Some((id, to, subject, html, text, attempts)) = row else {
        return Ok(false);
    };
    let mut body = serde_json::json!({
        "from": m.from, "to": [to], "subject": subject, "html": html, "text": text,
    });
    if let Some(r) = &m.reply_to {
        body["reply_to"] = r.as_str().into();
    }
    let res = m
        .http
        .post(format!("{}/emails", m.api_url))
        .bearer_auth(&m.api_key)
        .header("Idempotency-Key", id.to_string())
        .json(&body)
        .send()
        .await;
    let outcome = match res {
        Err(e) => Outcome::Retry(format!("network: {e}")),
        Ok(r) => {
            let status = r.status();
            let text: String = r
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(500)
                .collect();
            if status.is_success() {
                let pid = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| v["id"].as_str().map(str::to_string))
                    .unwrap_or_default();
                Outcome::Sent(pid)
            } else if status.as_u16() == 429 || status.as_u16() == 409 || status.is_server_error() {
                Outcome::Retry(format!("{status}: {text}"))
            } else {
                Outcome::Fatal(format!("{status}: {text}"))
            }
        }
    };
    let outcome = match outcome {
        Outcome::Retry(e) if attempts >= MAX_ATTEMPTS => Outcome::Fatal(e),
        o => o,
    };
    match outcome {
        Outcome::Sent(pid) => {
            sqlx::query(
                "UPDATE email_outbox SET status = 'sent', provider_id = $2, sent_at = now(), html = NULL, text_body = NULL, last_error = NULL WHERE id = $1",
            )
            .bind(id)
            .bind(pid)
            .execute(db)
            .await?;
            tracing::info!(email = %id, "email sent");
        }
        Outcome::Retry(e) => {
            let delay = m.retry_base.as_secs_f64() * 2f64.powi(attempts - 1);
            sqlx::query(
                "UPDATE email_outbox SET next_attempt_at = now() + make_interval(secs => $2), last_error = $3 WHERE id = $1",
            )
            .bind(id)
            .bind(delay)
            .bind(&e)
            .execute(db)
            .await?;
            tracing::warn!(email = %id, attempts, "email send failed, will retry: {e}");
        }
        Outcome::Fatal(e) => {
            sqlx::query(
                "UPDATE email_outbox SET status = 'failed', html = NULL, text_body = NULL, last_error = $2 WHERE id = $1",
            )
            .bind(id)
            .bind(&e)
            .execute(db)
            .await?;
            tracing::error!(email = %id, attempts, "email failed permanently: {e}");
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn render_escapes_and_falls_back() {
        let r = super::render(
            "xx",
            "email_changed",
            "<b>Ada</b>",
            "a@b.co",
            &[("new_email", "n@b.co")],
            "https://app/x?a=1&b=2",
        );
        assert_eq!(r.subject, "Your glot email address was changed");
        assert!(r.html.contains("&lt;b&gt;Ada&lt;/b&gt;") && !r.html.contains("<b>Ada"));
        assert!(r.html.contains("https://app/x?a=1&amp;b=2"));
        assert!(r.text.contains("n@b.co") && r.text.contains("https://app/x?a=1&b=2"));
        assert!(!r.text.contains('{'));
    }
}
