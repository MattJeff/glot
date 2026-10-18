use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use glot_core::catalog::{Catalog, Format};

use crate::{ApiResult, AppError, AppPath, AppState, auth::authenticate};

/// `GET /cdn/{slug}/{locale}.json`: nested JSON catalog for OTA updates.
/// Public projects (public_cdn) are readable by anyone; private ones only by members.
pub async fn serve(
    State(state): State<AppState>,
    AppPath((slug, file)): AppPath<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let not_found = || AppError::not_found("catalog not found");
    let locale = file.strip_suffix(".json").ok_or_else(not_found)?;
    let p = crate::projects::load_public(&state, &slug)
        .await?
        .ok_or_else(not_found)?;
    if !p.public_cdn {
        let user = authenticate(&headers, &state)
            .await?
            .ok_or_else(not_found)?;
        crate::projects::load(&state, user.id, &slug, crate::projects::Role::Translator).await?;
    }
    if locale != p.source_locale && !p.target_locales.iter().any(|l| l == locale) {
        return Err(not_found());
    }

    let rows: Vec<(String, String)> = if locale == p.source_locale {
        sqlx::query_as("SELECT key, source FROM keys WHERE project_id = $1")
            .bind(p.id)
            .fetch_all(&state.db)
            .await?
    } else {
        sqlx::query_as("SELECT k.key, t.value FROM translations t JOIN keys k ON k.id = t.key_id WHERE k.project_id = $1 AND t.locale = $2")
            .bind(p.id)
            .bind(locale)
            .fetch_all(&state.db)
            .await?
    };
    let mut catalog = Catalog::new(locale);
    catalog.entries.extend(rows);
    let body = catalog.to_json(Format::JsonNested);
    let etag = format!("\"{}\"", glot_core::hash::hash(&body));

    let cache = if p.public_cdn {
        "public, max-age=60, stale-while-revalidate=600"
    } else {
        "private, no-cache"
    };
    let mut res = if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(etag.as_str())
    {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            body,
        )
            .into_response()
    };
    let h = res.headers_mut();
    h.insert(header::ETAG, HeaderValue::from_str(&etag).expect("etag"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if p.public_cdn {
        h.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
    }
    Ok(res)
}
