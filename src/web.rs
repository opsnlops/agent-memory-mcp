//! A read-only web browser for the memory store.
//!
//! `/` lists projects, `/p/<project>` lists a project's entries (`/general`
//! for entries without one), `/m/<id>` reads one entry, and `/search?q=`
//! searches everything. Pages are rendered on the server with no JavaScript
//! needed; the only script turns UTC timestamps into the viewer's local time.

use axum::{
    Router,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use maud::{DOCTYPE, Markup, PreEscaped, html};
use mongodb::bson::{self, oid::ObjectId};
use serde::Deserialize;

use crate::{
    markdown,
    store::{Kind, MemoryDoc, ProjectSummary, SearchQuery, Store},
};

const STYLE: &str = include_str!("web/style.css");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const EXCERPT_CHARS: usize = 200;
const SEARCH_LIMIT: i64 = 50;
const PROJECT_STATE_TAG: &str = "project-state";

/// The browser's routes, guarded by the same Host allow-list as `/mcp`
/// (`None` accepts any host).
pub fn router(store: Store, allowed_hosts: Option<Vec<String>>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/general", get(general))
        .route("/p/{project}", get(project))
        .route("/m/{id}", get(memory))
        .route("/search", get(search))
        .route("/static/style.css", get(style))
        .fallback(|| async { not_found("There's nothing at this address.") })
        .layer(middleware::from_fn(move |req: Request, next: Next| {
            let allowed = allowed_hosts.clone();
            async move {
                if host_allowed(req.headers(), allowed.as_deref()) {
                    next.run(req).await
                } else {
                    (
                        StatusCode::FORBIDDEN,
                        "Forbidden: Host header is not allowed",
                    )
                        .into_response()
                }
            }
        }))
        .with_state(store)
}

/// Same rule as rmcp applies to `/mcp`: the Host header, without its port,
/// must be on the allow-list. This stops DNS-rebinding pages from reading
/// memories through a victim's browser.
fn host_allowed(headers: &HeaderMap, allowed: Option<&[String]>) -> bool {
    let Some(allowed) = allowed else {
        return true;
    };
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else {
        return false;
    };
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host.rsplit_once(':').map_or(host, |(name, port)| {
            if port.bytes().all(|b| b.is_ascii_digit()) {
                name
            } else {
                host
            }
        })
    };
    allowed.iter().any(|a| a.eq_ignore_ascii_case(name))
}

// ---------- handlers ----------

async fn style() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        STYLE,
    )
}

async fn index(State(store): State<Store>) -> Result<Page, Response> {
    let projects = store.project_summaries().await.map_err(server_error)?;
    let recent = store
        .search(SearchQuery {
            limit: 8,
            ..Default::default()
        })
        .await
        .map_err(server_error)?;
    let total: i64 = projects.iter().map(|p| p.count).sum();
    let named = projects.iter().filter(|p| p.project.is_some()).count();

    Ok(page(
        "Projects",
        html! {
            header.page-head {
                h1 { "Projects" }
                p.lede { (plural(total, "entry", "entries")) " across " (plural(named as i64, "project", "projects")) "." }
            }
            ul.project-grid {
                @for p in &projects { li { (project_card(p)) } }
            }
            @if !recent.is_empty() {
                section.recent {
                    h2.section-label { "Recently updated" }
                    ul.entry-list.compact {
                        @for m in &recent { li { (entry_row(m, true)) } }
                    }
                }
            }
        },
        None,
    ))
}

async fn general(State(store): State<Store>) -> Result<Page, Response> {
    project_page(&store, None).await
}

async fn project(
    State(store): State<Store>,
    Path(project): Path<String>,
) -> Result<Page, Response> {
    project_page(&store, Some(&project)).await
}

async fn project_page(store: &Store, project: Option<&str>) -> Result<Page, Response> {
    let entries = store.list_project(project).await.map_err(server_error)?;
    if entries.is_empty() {
        return Err(not_found("No entries in this project."));
    }
    let (state, rest): (Vec<&MemoryDoc>, Vec<&MemoryDoc>) = entries
        .iter()
        .partition(|m| m.tags.iter().any(|t| t == PROJECT_STATE_TAG));
    let (techniques, memories): (Vec<&MemoryDoc>, Vec<&MemoryDoc>) =
        rest.into_iter().partition(|m| m.kind == Kind::Technique);
    let name = project.unwrap_or("General");

    Ok(page(
        name,
        html! {
            nav.crumbs { a href="/" { "Projects" } }
            header.page-head {
                h1 { (name) }
                p.lede {
                    @if project.is_none() { "Knowledge that applies everywhere. " }
                    (plural(entries.len() as i64, "entry", "entries"))
                    ", last updated " (when(entries[0].updated_at)) "."
                }
            }
            @for m in &state {
                a.start-here href=(memory_url(m)) {
                    span.start-label { "Start here" }
                    span.start-title { (m.title) }
                    span.start-excerpt { (markdown::excerpt(&m.body, EXCERPT_CHARS)) }
                }
            }
            @if !techniques.is_empty() {
                section {
                    h2.section-label { "Techniques" span.count { (techniques.len()) } }
                    ul.entry-list { @for m in &techniques { li { (entry_row(m, false)) } } }
                }
            }
            @if !memories.is_empty() {
                section {
                    h2.section-label { "Memories" span.count { (memories.len()) } }
                    ul.entry-list { @for m in &memories { li { (entry_row(m, false)) } } }
                }
            }
        },
        None,
    ))
}

async fn memory(State(store): State<Store>, Path(id): Path<String>) -> Result<Page, Response> {
    let oid = ObjectId::parse_str(&id).map_err(|_| not_found("That isn't an entry id."))?;
    let m = store
        .get(oid)
        .await
        .map_err(server_error)?
        .ok_or_else(|| not_found("No entry with that id. It may have been forgotten."))?;
    let rendered = markdown::render(&m.body);
    let toc: Vec<_> = rendered.toc.iter().filter(|t| t.level <= 3).collect();
    let min_level = toc.iter().map(|t| t.level).min().unwrap_or(2);
    let show_toc = toc.len() >= 3;

    Ok(page(
        &m.title,
        html! {
            nav.crumbs {
                a href="/" { "Projects" }
                span.sep { "/" }
                a href=(project_url(m.project.as_deref())) { (m.project.as_deref().unwrap_or("General")) }
            }
            div.reader.has-toc[show_toc] {
                article.entry {
                    header.entry-head {
                        p.eyebrow { (kind_badge(m.kind)) }
                        h1 { (m.title) }
                        p.meta {
                            "Updated " (when(m.updated_at))
                            @if m.updated_at != m.created_at { span.dot { "·" } "created " (when(m.created_at)) }
                            @if let Some(a) = &m.author { span.dot { "·" } "by " (a) }
                        }
                        @if !m.tags.is_empty() {
                            ul.tags { @for t in &m.tags { li { (t) } } }
                        }
                    }
                    div.prose { (PreEscaped(&rendered.html)) }
                    footer.entry-foot {
                        span { "Entry id " code { (m.id.to_hex()) } }
                    }
                }
                @if show_toc {
                    aside.toc aria-label="Contents" {
                        p.toc-label { "Contents" }
                        ol {
                            @for t in &toc {
                                li class={ "lvl" (t.level - min_level) } { a href={ "#" (t.id) } { (t.text) } }
                            }
                        }
                    }
                }
            }
        },
        Some(&markdown::excerpt(&m.body, 160)),
    ))
}

#[derive(Deserialize)]
struct SearchForm {
    #[serde(default)]
    q: String,
}

async fn search(
    State(store): State<Store>,
    Query(form): Query<SearchForm>,
) -> Result<Page, Response> {
    let q = form.q.trim();
    let results = if q.is_empty() {
        Vec::new()
    } else {
        store
            .search(SearchQuery {
                text: Some(q.to_string()),
                limit: SEARCH_LIMIT,
                ..Default::default()
            })
            .await
            .map_err(server_error)?
    };
    Ok(page(
        if q.is_empty() { "Search" } else { q },
        html! {
            nav.crumbs { a href="/" { "Projects" } }
            header.page-head {
                h1 { "Search" }
                @if !q.is_empty() {
                    p.lede {
                        (plural(results.len() as i64, "result", "results")) " for “" (q) "”."
                        @if results.is_empty() { " Search matches whole words, so try another form of the word." }
                    }
                }
            }
            @if !results.is_empty() {
                ul.entry-list { @for m in &results { li { (entry_row(m, true)) } } }
            }
        },
        None,
    )
    .with_query(q))
}

// ---------- fragments ----------

fn project_card(p: &ProjectSummary) -> Markup {
    html! {
        a.project-card href=(project_url(p.project.as_deref())) {
            span.project-name { (p.project.as_deref().unwrap_or("General")) }
            span.project-stats {
                (plural(p.count, "entry", "entries"))
                @if p.techniques > 0 { span.dot { "·" } (plural(p.techniques, "technique", "techniques")) }
            }
            span.project-when { "Updated " (when(p.last_updated)) }
        }
    }
}

fn entry_row(m: &MemoryDoc, show_project: bool) -> Markup {
    html! {
        a.entry-row href=(memory_url(m)) {
            span.entry-title { (m.title) }
            span.entry-excerpt { (markdown::excerpt(&m.body, EXCERPT_CHARS)) }
            span.entry-meta {
                (kind_badge(m.kind))
                @if show_project {
                    span.entry-project { (m.project.as_deref().unwrap_or("General")) }
                }
                @for t in m.tags.iter().take(4) { span.tag { (t) } }
                span.entry-when { (when(m.updated_at)) }
            }
        }
    }
}

fn kind_badge(kind: Kind) -> Markup {
    match kind {
        Kind::Technique => html! { span.badge.technique { "Technique" } },
        Kind::Memory => html! { span.badge.memory { "Memory" } },
    }
}

/// A `<time>` showing the UTC date; the page script rewrites it to local time.
fn when(t: bson::DateTime) -> Markup {
    let iso = t.try_to_rfc3339_string().unwrap_or_default();
    html! { time datetime=(iso) title=(iso) { (format_date(t)) } }
}

fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn memory_url(m: &MemoryDoc) -> String {
    format!("/m/{}", m.id.to_hex())
}

fn project_url(project: Option<&str>) -> String {
    match project {
        Some(p) => format!("/p/{}", encode_path_segment(p)),
        None => "/general".to_string(),
    }
}

fn encode_path_segment(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// "Nov 12, 2026" in UTC, from days since the Unix epoch (Hinnant's algorithm).
fn format_date(t: bson::DateTime) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = t.timestamp_millis().div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{} {day}, {year}", MONTHS[(month - 1) as usize])
}

// ---------- page shell ----------

pub struct Page {
    title: String,
    body: Markup,
    description: Option<String>,
    query: String,
}

fn page(title: &str, body: Markup, description: Option<&str>) -> Page {
    Page {
        title: title.to_string(),
        body,
        description: description.map(String::from),
        query: String::new(),
    }
}

impl Page {
    fn with_query(mut self, q: &str) -> Self {
        self.query = q.to_string();
        self
    }

    fn render(self) -> String {
        html! {
            (DOCTYPE)
            html lang="en" {
                head {
                    meta charset="utf-8";
                    meta name="viewport" content="width=device-width, initial-scale=1";
                    meta name="color-scheme" content="light dark";
                    @if let Some(d) = &self.description { meta name="description" content=(d); }
                    title { (self.title) " · agent-memory" }
                    link rel="icon" href=(FAVICON);
                    link rel="stylesheet" href={ "/static/style.css?v=" (VERSION) };
                }
                body {
                    header.site {
                        div.site-inner {
                            a.wordmark href="/" { span.mark { "◆" } "agent-memory" }
                            form.search action="/search" method="get" role="search" {
                                input type="search" name="q" value=(self.query) placeholder="Search memories" aria-label="Search memories";
                            }
                        }
                    }
                    main.site-inner { (self.body) }
                    footer.site-foot { div.site-inner { "agent-memory " (VERSION) " · read-only" } }
                    script { (PreEscaped(LOCAL_TIME_JS)) }
                }
            }
        }
        .into_string()
    }
}

impl IntoResponse for Page {
    fn into_response(self) -> Response {
        Html(self.render()).into_response()
    }
}

const FAVICON: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'%3E%3Cpath d='M8 1l7 7-7 7-7-7z' fill='%232f5d8a'/%3E%3C/svg%3E";

/// Show dates in the viewer's time zone. Without it, pages show UTC dates.
const LOCAL_TIME_JS: &str = r#"
for (const t of document.querySelectorAll('time[datetime]')) {
  const d = new Date(t.getAttribute('datetime'));
  if (isNaN(d)) continue;
  t.textContent = d.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  t.title = d.toLocaleString(undefined, { dateStyle: 'full', timeStyle: 'short' });
}
"#;

// ---------- errors ----------

fn not_found(message: &str) -> Response {
    let body = html! {
        nav.crumbs { a href="/" { "Projects" } }
        header.page-head { h1 { "Not found" } p.lede { (message) } }
    };
    (
        StatusCode::NOT_FOUND,
        Html(page("Not found", body, None).render()),
    )
        .into_response()
}

fn server_error(err: anyhow::Error) -> Response {
    tracing::error!("web: {err:#}");
    let body = html! {
        header.page-head { h1 { "Something went wrong" } p.lede { "The server couldn't read from MongoDB. Details are in its log." } }
    };
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(page("Error", body, None).render()),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn hosts(h: &str) -> HeaderMap {
        let mut m = HeaderMap::new();
        m.insert(header::HOST, HeaderValue::from_str(h).unwrap());
        m
    }

    #[test]
    fn host_check_strips_ports_and_brackets() {
        let allowed = vec!["localhost".to_string(), "::1".into(), "fuzzball".into()];
        for ok in ["localhost:8787", "fuzzball", "FUZZBALL:80", "[::1]:8787"] {
            assert!(host_allowed(&hosts(ok), Some(&allowed)), "{ok}");
        }
        for bad in ["evil.example", "evil.example:8787", "fuzzball.evil"] {
            assert!(!host_allowed(&hosts(bad), Some(&allowed)), "{bad}");
        }
        assert!(!host_allowed(&HeaderMap::new(), Some(&allowed)));
        assert!(host_allowed(&hosts("anything"), None));
    }

    #[test]
    fn dates_format_in_utc() {
        let t = bson::DateTime::from_millis(1_794_539_302_000); // 2026-11-13T02:28:22Z
        assert_eq!(format_date(t), "Nov 13, 2026");
        assert_eq!(format_date(bson::DateTime::from_millis(0)), "Jan 1, 1970");
        assert_eq!(
            format_date(bson::DateTime::from_millis(951_782_400_000)),
            "Feb 29, 2000"
        );
    }

    #[test]
    fn project_urls_are_encoded() {
        assert_eq!(project_url(Some("agent-memory-mcp")), "/p/agent-memory-mcp");
        assert_eq!(project_url(Some("a b/c")), "/p/a%20b%2Fc");
        assert_eq!(project_url(None), "/general");
    }
}
