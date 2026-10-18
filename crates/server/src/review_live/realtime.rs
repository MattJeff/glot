//! Live review SSE. Writers `pg_notify('glot_review', …)` inside their transaction (model::emit); one LISTEN
//! connection per process fans the events out to the SSE streams of their project, across replicas.
use axum::{
    extract::State,
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use serde::Deserialize;
use sqlx::{PgPool, postgres::PgListener};
use std::{
    convert::Infallible,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::broadcast;
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use uuid::Uuid;

use super::model::{Ctx, EVENT_COLS, EventRow, ReviewEvent};
use crate::{ApiResult, AppQuery, AppState};

#[derive(Deserialize)]
pub struct Wire {
    project_id: Uuid,
    event: ReviewEvent,
}

// ponytail: one process-wide listener bound to the first pool that starts it (one database per process).
static HUB: OnceLock<broadcast::Sender<Arc<Wire>>> = OnceLock::new();

/// Starts the LISTEN task once (needs a Tokio runtime).
pub fn hub(db: &PgPool) -> &'static broadcast::Sender<Arc<Wire>> {
    HUB.get_or_init(|| {
        let (tx, _) = broadcast::channel(1024);
        tokio::spawn(listen(db.clone(), tx.clone()));
        tx
    })
}

async fn listen(db: PgPool, tx: broadcast::Sender<Arc<Wire>>) {
    loop {
        match PgListener::connect_with(&db).await {
            Ok(mut l) => match l.listen("glot_review").await {
                // recv() reconnects by itself; notifications sent while reconnecting are recovered by
                // clients through Last-Event-ID.
                Ok(()) => loop {
                    match l.recv().await {
                        Ok(n) => {
                            if let Ok(w) = serde_json::from_str::<Wire>(n.payload()) {
                                let _ = tx.send(Arc::new(w));
                            }
                        }
                        Err(e) => {
                            tracing::warn!("review listener: {e}");
                            break;
                        }
                    }
                },
                Err(e) => tracing::warn!("review listen: {e}"),
            },
            Err(e) => {
                if db.is_closed() {
                    return;
                }
                tracing::warn!("review listener connect: {e}");
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[derive(Deserialize)]
pub struct Since {
    since: Option<i64>,
}

fn sse_event(e: &ReviewEvent) -> Event {
    Event::default()
        .event("review")
        .id(e.id.to_string())
        .json_data(e)
        .expect("review event json")
}

/// `GET …/events`: replays from `Last-Event-ID` / `?since=` (≤ 500), then streams the project's events.
// ponytail: a revoked link keeps an already-open stream until it reconnects.
pub async fn events(
    State(state): State<AppState>,
    ctx: Ctx,
    headers: HeaderMap,
    AppQuery(q): AppQuery<Since>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    // Subscribe before reading the backlog so nothing falls in between.
    let mut rx = hub(&state.db).subscribe();
    let since = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok()?.parse::<i64>().ok())
        .or(q.since);
    let project = ctx.project.id;
    let backlog: Vec<ReviewEvent> = match since {
        Some(s) => {
            let rows: Vec<EventRow> = sqlx::query_as(&format!(
                "SELECT {EVENT_COLS} FROM review_events WHERE project_id = $1 AND id > $2 ORDER BY id LIMIT 500"
            ))
            .bind(project)
            .bind(s)
            .fetch_all(&state.db)
            .await?;
            rows.into_iter().map(ReviewEvent::from).collect()
        }
        None => vec![],
    };
    let (tx, out) = tokio::sync::mpsc::channel::<Event>(64);
    tokio::spawn(async move {
        let mut last = since.unwrap_or(0);
        for e in &backlog {
            if tx.send(sse_event(e)).await.is_err() {
                return;
            }
            last = e.id;
        }
        loop {
            match rx.recv().await {
                Ok(w) if w.project_id == project && w.event.id > last => {
                    if tx.send(sse_event(&w.event)).await.is_err() {
                        return;
                    }
                    last = w.event.id;
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return,
            }
        }
    });
    Ok(Sse::new(ReceiverStream::new(out).map(Ok))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
