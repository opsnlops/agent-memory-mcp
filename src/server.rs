//! MCP tool surface over the memory store.

use mongodb::bson::oid::ObjectId;
use rmcp::{
    ErrorData, ServerHandler,
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};

use crate::store::{Kind, MemoryDoc, MemoryPatch, NewMemory, SearchQuery, Store, TagCount};

const DEFAULT_LIMIT: i64 = 10;
const MAX_LIMIT: i64 = 50;
const SNIPPET_CHARS: usize = 240;

#[derive(Clone)]
pub struct MemoryServer {
    store: Store,
}

impl MemoryServer {
    pub fn new(store: Store) -> Self {
        Self { store }
    }
}

// ---------- tool inputs ----------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RememberParams {
    /// Short, searchable headline, e.g. "Use cargo nextest for faster test runs".
    pub title: String,
    /// The full memory or technique. Markdown is fine. Include enough context
    /// that an agent in a different session can act on it.
    pub body: String,
    /// "memory" for facts and context, "technique" for reusable how-tos. Defaults to "memory".
    #[serde(default)]
    pub kind: Option<Kind>,
    /// Lowercase topic tags, e.g. ["rust", "testing"].
    #[serde(default)]
    pub tags: Vec<String>,
    /// Project or repo this applies to. Omit for knowledge that applies everywhere.
    #[serde(default)]
    pub project: Option<String>,
    /// Who wrote it, e.g. "claude-code" or "codex".
    #[serde(default)]
    pub author: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Free-text query matched against title, tags, and body. Omit to list the most recent entries.
    #[serde(default)]
    pub query: Option<String>,
    /// Only return entries that have all of these tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Only return this kind of entry.
    #[serde(default)]
    pub kind: Option<Kind>,
    /// Only return entries for this project.
    #[serde(default)]
    pub project: Option<String>,
    /// Maximum results (default 10, max 50).
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IdParams {
    /// The entry's id, as returned by remember or search.
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateParams {
    /// The entry's id.
    pub id: String,
    /// New title, if changing.
    #[serde(default)]
    pub title: Option<String>,
    /// New body, if changing. Replaces the whole body.
    #[serde(default)]
    pub body: Option<String>,
    /// New kind, if changing.
    #[serde(default)]
    pub kind: Option<Kind>,
    /// New tags, if changing. Replaces the whole tag list.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// New project, if changing.
    #[serde(default)]
    pub project: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListTagsParams {
    /// Only count tags on entries for this project.
    #[serde(default)]
    pub project: Option<String>,
}

// ---------- tool outputs ----------

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct Memory {
    pub id: String,
    pub title: String,
    pub body: String,
    pub kind: Kind,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<MemoryDoc> for Memory {
    fn from(d: MemoryDoc) -> Self {
        Self {
            id: d.id.to_hex(),
            title: d.title,
            body: d.body,
            kind: d.kind,
            tags: d.tags,
            project: d.project,
            author: d.author,
            created_at: d.created_at.try_to_rfc3339_string().unwrap_or_default(),
            updated_at: d.updated_at.try_to_rfc3339_string().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    pub kind: Kind,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub updated_at: String,
    /// First part of the body; call `get` for the full text.
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

impl From<MemoryDoc> for SearchHit {
    fn from(d: MemoryDoc) -> Self {
        let mut snippet: String = d.body.chars().take(SNIPPET_CHARS).collect();
        if d.body.chars().count() > SNIPPET_CHARS {
            snippet.push('…');
        }
        Self {
            id: d.id.to_hex(),
            title: d.title,
            kind: d.kind,
            tags: d.tags,
            project: d.project,
            author: d.author,
            updated_at: d.updated_at.try_to_rfc3339_string().unwrap_or_default(),
            snippet,
            score: d.score,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchResults {
    pub results: Vec<SearchHit>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ForgetResult {
    pub deleted: bool,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TagList {
    pub tags: Vec<TagCount>,
}

// ---------- helpers ----------

fn internal(err: anyhow::Error) -> ErrorData {
    tracing::error!("{err:#}");
    ErrorData::internal_error(format!("{err:#}"), None)
}

fn parse_id(id: &str) -> Result<ObjectId, ErrorData> {
    ObjectId::parse_str(id.trim())
        .map_err(|_| ErrorData::invalid_params(format!("invalid id: {id:?}"), None))
}

fn not_found(id: &str) -> ErrorData {
    ErrorData::resource_not_found(format!("no memory with id {id}"), None)
}

fn require_nonempty(field: &str, value: &str) -> Result<(), ErrorData> {
    if value.trim().is_empty() {
        Err(ErrorData::invalid_params(
            format!("{field} must not be empty"),
            None,
        ))
    } else {
        Ok(())
    }
}

// ---------- tools ----------

#[tool_router]
impl MemoryServer {
    #[tool(
        description = "Save a memory or technique so other agent sessions can find it later. \
                       Search first to avoid duplicates; update an existing entry instead of re-adding it."
    )]
    async fn remember(
        &self,
        Parameters(p): Parameters<RememberParams>,
    ) -> Result<Json<Memory>, ErrorData> {
        require_nonempty("title", &p.title)?;
        require_nonempty("body", &p.body)?;
        let doc = self
            .store
            .insert(NewMemory {
                title: p.title.trim().to_string(),
                body: p.body,
                kind: p.kind.unwrap_or(Kind::Memory),
                tags: p.tags,
                project: p.project,
                author: p.author,
            })
            .await
            .map_err(internal)?;
        Ok(Json(doc.into()))
    }

    #[tool(
        description = "Search shared memories and techniques by free text, tags, kind, or project. \
                       Returns short snippets; call get for the full body.",
        annotations(read_only_hint = true)
    )]
    async fn search(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<Json<SearchResults>, ErrorData> {
        let docs = self
            .store
            .search(SearchQuery {
                text: p.query,
                tags: p.tags,
                kind: p.kind,
                project: p.project,
                limit: p.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
            })
            .await
            .map_err(internal)?;
        Ok(Json(SearchResults {
            results: docs.into_iter().map(Into::into).collect(),
        }))
    }

    #[tool(
        description = "Fetch one memory by id, including its full body.",
        annotations(read_only_hint = true)
    )]
    async fn get(&self, Parameters(p): Parameters<IdParams>) -> Result<Json<Memory>, ErrorData> {
        let id = parse_id(&p.id)?;
        match self.store.get(id).await.map_err(internal)? {
            Some(doc) => Ok(Json(doc.into())),
            None => Err(not_found(&p.id)),
        }
    }

    #[tool(description = "Change an existing memory. Only the fields you pass are changed.")]
    async fn update(
        &self,
        Parameters(p): Parameters<UpdateParams>,
    ) -> Result<Json<Memory>, ErrorData> {
        let id = parse_id(&p.id)?;
        if let Some(title) = &p.title {
            require_nonempty("title", title)?;
        }
        if let Some(body) = &p.body {
            require_nonempty("body", body)?;
        }
        let patch = MemoryPatch {
            title: p.title.map(|t| t.trim().to_string()),
            body: p.body,
            kind: p.kind,
            tags: p.tags,
            project: p.project,
        };
        match self.store.update(id, patch).await.map_err(internal)? {
            Some(doc) => Ok(Json(doc.into())),
            None => Err(not_found(&p.id)),
        }
    }

    #[tool(
        description = "Permanently delete a memory that is wrong or obsolete.",
        annotations(destructive_hint = true)
    )]
    async fn forget(
        &self,
        Parameters(p): Parameters<IdParams>,
    ) -> Result<Json<ForgetResult>, ErrorData> {
        let id = parse_id(&p.id)?;
        let deleted = self.store.delete(id).await.map_err(internal)?;
        Ok(Json(ForgetResult { deleted }))
    }

    #[tool(
        description = "List tags in use with how many entries carry each, most used first. \
                       Useful for discovering what topics exist before searching or tagging.",
        annotations(read_only_hint = true)
    )]
    async fn list_tags(
        &self,
        Parameters(p): Parameters<ListTagsParams>,
    ) -> Result<Json<TagList>, ErrorData> {
        let tags = self.store.tag_counts(p.project).await.map_err(internal)?;
        Ok(Json(TagList { tags }))
    }
}

#[tool_handler(
    name = "agent-memory",
    instructions = "Shared long-term memory for coding agents. Before starting non-trivial work, \
                    search for relevant memories and techniques. When you learn something another \
                    session would benefit from (a fix, a gotcha, a better way of doing things), \
                    remember it with a clear title and tags. Prefer updating an existing entry to \
                    adding a near-duplicate."
)]
impl ServerHandler for MemoryServer {}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson;

    fn doc_with_body(body: &str) -> MemoryDoc {
        let now = bson::DateTime::now();
        MemoryDoc {
            id: ObjectId::new(),
            title: "title".into(),
            body: body.into(),
            kind: Kind::Technique,
            tags: vec!["rust".into()],
            project: Some("proj".into()),
            author: None,
            created_at: now,
            updated_at: now,
            score: Some(2.0),
        }
    }

    #[test]
    fn short_body_is_not_truncated() {
        let hit = SearchHit::from(doc_with_body("short body"));
        assert_eq!(hit.snippet, "short body");
        assert_eq!(hit.score, Some(2.0));
    }

    #[test]
    fn long_body_is_truncated_on_char_boundary() {
        // Multi-byte characters must not be split.
        let body = "é".repeat(SNIPPET_CHARS + 10);
        let hit = SearchHit::from(doc_with_body(&body));
        assert_eq!(hit.snippet.chars().count(), SNIPPET_CHARS + 1);
        assert!(hit.snippet.ends_with('…'));
    }

    #[test]
    fn body_of_exactly_snippet_length_has_no_ellipsis() {
        let body = "a".repeat(SNIPPET_CHARS);
        assert_eq!(SearchHit::from(doc_with_body(&body)).snippet, body);
    }

    #[test]
    fn memory_conversion_uses_hex_id_and_rfc3339() {
        let doc = doc_with_body("b");
        let hex = doc.id.to_hex();
        let m = Memory::from(doc);
        assert_eq!(m.id, hex);
        assert!(m.created_at.ends_with('Z'), "{}", m.created_at);
        assert_eq!(m.project.as_deref(), Some("proj"));
    }

    #[test]
    fn parse_id_accepts_hex_with_whitespace() {
        let id = ObjectId::new();
        assert_eq!(parse_id(&format!("  {}\n", id.to_hex())).unwrap(), id);
    }

    #[test]
    fn parse_id_rejects_garbage() {
        let err = parse_id("nope").unwrap_err();
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn require_nonempty_rejects_whitespace() {
        assert!(require_nonempty("title", "  \n").is_err());
        assert!(require_nonempty("title", "x").is_ok());
    }
}
