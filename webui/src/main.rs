//! 本地 WebUI：浏览专题、选中装订、看进度、下载 EPUB。
//! 用法：`anitama-epub-web [归档根目录]`（默认当前目录），监听 127.0.0.1:8091。
mod archive;
mod cover;
mod epub;
mod html;
mod jobs;
mod util;

use anyhow::Result;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
struct App {
    arc: Arc<RwLock<Arc<archive::Archive>>>,
    jobs: jobs::Jobs,
}

#[derive(Deserialize)]
struct CoverQ {
    w: Option<u32>,
}

fn d_true() -> bool {
    true
}

#[derive(Deserialize)]
struct StartBody {
    sids: Vec<String>,
    #[serde(default = "d_true")]
    embed_images: bool,
    #[serde(default = "d_true")]
    comments: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,anitama_epub_web=debug".into()),
        )
        .init();

    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    let static_dir = std::env::var("STATIC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let a = root.join("webui/static");
            if a.exists() {
                a
            } else {
                PathBuf::from("static")
            }
        });

    tracing::info!("扫描归档：{}", root.display());
    let arc = archive::Archive::load(&root)?;
    tracing::info!(
        "专题 {} 个 · 素材 {} 个 · 静态目录 {}",
        arc.series.len(),
        arc.assets,
        static_dir.display()
    );

    let app = App {
        arc: Arc::new(RwLock::new(arc)),
        jobs: jobs::Jobs::new(),
    };

    let router = Router::new()
        .route("/api/state", get(api_state))
        .route("/api/series/:sid", get(api_series))
        .route("/api/cover/:sid", get(api_cover))
        .route("/api/read/:aid", get(api_read))
        .route("/media/*path", get(api_media))
        .route("/api/epubs", get(api_epubs))
        .route("/api/jobs", get(api_jobs).post(api_start))
        .route("/api/jobs/:id", get(api_job))
        .route("/api/jobs/:id/stream", get(api_stream))
        .route("/api/rescan", post(api_rescan))
        .route("/epub/:name", get(api_download))
        .fallback_service(
            ServeDir::new(static_dir.clone()).not_found_service(ServeFile::new(
                static_dir.join("index.html"),
            )),
        )
        .with_state(app);

    let bind = std::env::var("BIND").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8095);
    let listener = tokio::net::TcpListener::bind((bind.as_str(), port)).await?;
    tracing::info!("装订台已就绪 → http://{bind}:{port}/");
    axum::serve(listener, router).await?;
    Ok(())
}

async fn api_state(State(app): State<App>) -> Response {
    let arc = app.arc.read().await.clone();
    let covers = arc
        .series
        .iter()
        .filter(|s| arc.cover_path(&s.sid).is_some())
        .count();
    let articles: usize = arc.series.iter().map(|s| s.n).sum();
    let epubs = arc.epubs();
    let series: Vec<_> = arc
        .series
        .iter()
        .map(|s| {
            json!({
                "sid": s.sid,
                "title": s.title,
                "title_text": s.title_text,
                "n": s.n,
                "first": s.items.first().map(|i| i.aid.clone()),
                "cover": arc.cover_path(&s.sid).is_some(),
                "authors": s.authors,
                "channel": s.channel,
                "y0": s.y0,
                "y1": s.y1,
            })
        })
        .collect();
    Json(json!({
        "stats": {
            "series": arc.series.len(),
            "covers": covers,
            "articles": articles,
            "assets": arc.assets,
            "generated": epubs.len(),
            "out": arc.out.to_string_lossy(),
        },
        "series": series,
        "jobs": app.jobs.list(),
        "epubs": epubs,
    }))
    .into_response()
}

async fn api_series(State(app): State<App>, AxPath(sid): AxPath<String>) -> Response {
    let arc = app.arc.read().await.clone();
    match arc.by_sid(&sid) {
        Some(s) => Json(json!({
            "sid": s.sid,
            "title": s.title,
            "title_text": s.title_text,
            "n": s.n,
            "cover": arc.cover_path(&s.sid).is_some(),
            "authors": s.authors,
            "channel": s.channel,
            "y0": s.y0,
            "y1": s.y1,
            "items": s.items,
        }))
        .into_response(),
        None => (StatusCode::NOT_FOUND, "no such series").into_response(),
    }
}

async fn api_cover(
    State(app): State<App>,
    AxPath(sid): AxPath<String>,
    Query(q): Query<CoverQ>,
) -> Response {
    let arc = app.arc.read().await.clone();
    let w = q.w.unwrap_or(660).clamp(80, 1600);
    let sid2 = sid.clone();
    let thumb = tokio::task::spawn_blocking(move || arc.thumb(&sid2, w))
        .await
        .ok()
        .flatten();
    match thumb {
        Some(bytes) => {
            let mut h = HeaderMap::new();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
            h.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400"),
            );
            (h, bytes.as_ref().clone()).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// 在线阅读：单篇文章的正文（图片指向 /media/*）。
async fn api_read(State(app): State<App>, AxPath(aid): AxPath<String>) -> Response {
    if aid.is_empty() || !aid.chars().all(|c| c.is_ascii_alphanumeric()) {
        return (StatusCode::BAD_REQUEST, "文章 id 不合法").into_response();
    }
    let arc = app.arc.read().await.clone();
    let site = arc.site.clone();
    let aid2 = aid.clone();
    let got = tokio::task::spawn_blocking(move || -> std::result::Result<serde_json::Value, String> {
        let p = site.join("article").join(format!("{aid2}.html"));
        let raw = std::fs::read_to_string(&p).map_err(|e| format!("读不到文章：{e}"))?;
        let art = html::parse_article(&raw);
        let body = html::rewrite_web(&art.content, &site).map_err(|e| format!("{e}"))?;
        Ok(json!({
            "aid": aid2,
            "title": art.title,
            "subtitle": art.subtitle,
            "author": art.author,
            "time": art.time_txt,
            "html": body,
        }))
    })
    .await;
    match got {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("内部错误：{e}")).into_response(),
    }
}

/// 归档里的图片/视频封面等素材，按原文件回给浏览器。
async fn api_media(State(app): State<App>, AxPath(path): AxPath<String>) -> Response {
    if path
        .split('/')
        .any(|s| s.is_empty() || s == "." || s == ".." || s.contains('\\'))
    {
        return (StatusCode::BAD_REQUEST, "路径不合法").into_response();
    }
    let decoded = match percent_encoding::percent_decode_str(&path).decode_utf8() {
        Ok(d) => d.into_owned(),
        Err(_) => return (StatusCode::BAD_REQUEST, "路径编码不合法").into_response(),
    };
    let arc = app.arc.read().await.clone();
    let full = arc.site.join(&decoded);
    match tokio::fs::read(&full).await {
        Ok(bytes) => {
            let (mime, _) = util::snifftype(&full);
            let mut h = HeaderMap::new();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
            h.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400"),
            );
            (h, bytes).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn api_epubs(State(app): State<App>) -> Response {
    let arc = app.arc.read().await.clone();
    Json(json!({ "epubs": arc.epubs() })).into_response()
}

async fn api_jobs(State(app): State<App>) -> Response {
    Json(json!({ "jobs": app.jobs.list() })).into_response()
}

async fn api_job(State(app): State<App>, AxPath(id): AxPath<String>) -> Response {
    match app.jobs.get(&id) {
        Some(j) => Json(json!({ "job": j })).into_response(),
        None => (StatusCode::NOT_FOUND, "没有这个任务").into_response(),
    }
}

async fn api_start(State(app): State<App>, Json(body): Json<StartBody>) -> Response {
    if body.sids.is_empty() {
        return (StatusCode::BAD_REQUEST, "没有选中专题").into_response();
    }
    let arc = app.arc.read().await.clone();
    let job = app
        .jobs
        .start(arc, body.sids, body.embed_images, body.comments);
    Json(json!({ "job": job })).into_response()
}

async fn api_stream(State(app): State<App>, AxPath(id): AxPath<String>) -> Response {
    let rx = app.jobs.subscribe();
    let current = app.jobs.get(&id);
    let first = current
        .map(|j| serde_json::to_string(&json!({ "job": j })).unwrap_or_default())
        .unwrap_or_default();
    let head = tokio_stream::once(Ok::<_, std::convert::Infallible>(
        Event::default().data(first),
    ));
    let tail = BroadcastStream::new(rx).filter_map(move |r| {
        let id = id.clone();
        async move {
            let s = r.ok()?;
            let v: serde_json::Value = serde_json::from_str(&s).ok()?;
            if v.get("job")?.get("id")?.as_str()? != id {
                return None;
            }
            Some(Ok(Event::default().data(s)))
        }
    });
    Sse::new(head.chain(tail))
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn api_rescan(State(app): State<App>) -> Response {
    let root = app.arc.read().await.root.clone();
    match tokio::task::spawn_blocking(move || archive::Archive::load(&root)).await {
        Ok(Ok(arc)) => {
            let n = arc.series.len();
            *app.arc.write().await = arc;
            Json(json!({ "series": n })).into_response()
        }
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

async fn api_download(State(app): State<App>, AxPath(name): AxPath<String>) -> Response {
    if name.contains('/') || name.contains("..") || !name.ends_with(".epub") {
        return (StatusCode::BAD_REQUEST, "非法文件名").into_response();
    }
    let arc = app.arc.read().await.clone();
    let path = arc.out.join(&name);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let enc: String = percent_encoding::utf8_percent_encode(
                &name,
                percent_encoding::NON_ALPHANUMERIC,
            )
            .to_string();
            let mut h = HeaderMap::new();
            h.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/epub+zip"),
            );
            h.insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&format!("attachment; filename*=UTF-8''{enc}"))
                    .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
            );
            (h, bytes).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "没有这个文件").into_response(),
    }
}
