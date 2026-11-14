//! End-to-end tests: real HTTP server, real MCP client, real MongoDB.
//!
//! Set `MONGODB_TEST_URI` (e.g. `mongodb://127.0.0.1:27017`) to run them; they
//! are skipped otherwise. Each test uses its own throwaway database and drops
//! it afterwards.

use std::net::SocketAddr;

use agent_memory_mcp::{HttpOptions, app, store::Store};
use mongodb::bson::oid::ObjectId;
use rmcp::{
    RoleClient, ServiceExt,
    model::CallToolRequestParams,
    service::{RunningService, ServiceError},
    transport::StreamableHttpClientTransport,
};
use serde_json::{Value, json};

struct Harness {
    addr: SocketAddr,
    client: RunningService<RoleClient, ()>,
    mongo_uri: String,
    db: String,
}

impl Harness {
    /// Returns `None` (and the test passes vacuously) when no test MongoDB is configured.
    async fn start() -> Option<Self> {
        Self::start_with(HttpOptions::default()).await
    }

    async fn start_with(opts: HttpOptions) -> Option<Self> {
        let Ok(mongo_uri) = std::env::var("MONGODB_TEST_URI") else {
            eprintln!("MONGODB_TEST_URI not set; skipping integration test");
            return None;
        };
        let db = format!("agent_memory_test_{}", ObjectId::new().to_hex());
        let store = Store::connect(&mongo_uri, &db)
            .await
            .expect("connect to test MongoDB");
        let (router, _shutdown) = app(store, &opts);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let transport = StreamableHttpClientTransport::from_uri(format!("http://{addr}/mcp"));
        let client = ().serve(transport).await.expect("MCP initialize");
        Some(Self {
            addr,
            client,
            mongo_uri,
            db,
        })
    }

    async fn call(&self, tool: &'static str, args: Value) -> Result<Value, ServiceError> {
        let Value::Object(args) = args else {
            panic!("tool args must be an object")
        };
        let result = self
            .client
            .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
            .await?;
        assert_ne!(
            result.is_error,
            Some(true),
            "{tool} returned an error: {result:?}"
        );
        Ok(result
            .structured_content
            .unwrap_or_else(|| panic!("{tool} returned no structured content")))
    }

    async fn ok(&self, tool: &'static str, args: Value) -> Value {
        self.call(tool, args)
            .await
            .unwrap_or_else(|e| panic!("{tool} failed: {e}"))
    }

    async fn remember(&self, args: Value) -> String {
        self.ok("remember", args).await["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn finish(self) {
        let _ = self.client.cancel().await;
        let client = mongodb::Client::with_uri_str(&self.mongo_uri)
            .await
            .unwrap();
        client.database(&self.db).drop().await.unwrap();
    }
}

fn titles(results: &Value) -> Vec<String> {
    results["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn lists_all_tools() {
    let Some(h) = Harness::start().await else {
        return;
    };
    let mut names: Vec<String> = h
        .client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "edit",
            "forget",
            "get",
            "list_tags",
            "remember",
            "search",
            "update"
        ]
    );
    h.finish().await;
}

#[tokio::test]
async fn remember_get_update_forget_roundtrip() {
    let Some(h) = Harness::start().await else {
        return;
    };

    let saved = h
        .ok(
            "remember",
            json!({
                "title": "  Use cargo nextest  ",
                "body": "cargo nextest run is faster than cargo test.",
                "kind": "technique",
                "tags": ["Rust", " testing ", "rust"],
                "author": "test"
            }),
        )
        .await;
    let id = saved["id"].as_str().unwrap().to_string();
    assert_eq!(saved["title"], "Use cargo nextest");
    assert_eq!(saved["kind"], "technique");
    assert_eq!(saved["tags"], json!(["rust", "testing"]));
    assert!(
        saved.get("body").is_none(),
        "writes don't echo the body back"
    );
    assert_eq!(saved["body_chars"], 44);

    let got = h.ok("get", json!({ "id": id })).await;
    assert_eq!(got["body"], "cargo nextest run is faster than cargo test.");
    assert_eq!(got["created_at"], got["updated_at"]);
    assert_eq!(got["updated_at"], saved["updated_at"]);

    let updated = h
        .ok(
            "update",
            json!({ "id": id, "tags": ["cargo", "rust"], "project": "demo" }),
        )
        .await;
    assert_eq!(updated["tags"], json!(["cargo", "rust"]));
    assert_eq!(updated["project"], "demo");
    assert_eq!(
        updated["body_chars"], saved["body_chars"],
        "untouched fields keep their value"
    );
    assert_ne!(updated["updated_at"], saved["updated_at"]);

    assert_eq!(h.ok("forget", json!({ "id": id })).await["deleted"], true);
    assert_eq!(h.ok("forget", json!({ "id": id })).await["deleted"], false);
    assert!(h.call("get", json!({ "id": id })).await.is_err());

    h.finish().await;
}

#[tokio::test]
async fn kind_defaults_to_memory() {
    let Some(h) = Harness::start().await else {
        return;
    };
    let saved = h.ok("remember", json!({ "title": "t", "body": "b" })).await;
    assert_eq!(saved["kind"], "memory");
    assert_eq!(saved["tags"], json!([]));
    h.finish().await;
}

#[tokio::test]
async fn text_search_ranks_title_matches_first() {
    let Some(h) = Harness::start().await else {
        return;
    };
    h.remember(json!({ "title": "Unrelated note", "body": "Mentions postgres once." }))
        .await;
    h.remember(
        json!({ "title": "Postgres vacuum tuning", "body": "Set autovacuum scale factors." }),
    )
    .await;
    h.remember(json!({ "title": "Redis eviction", "body": "Use allkeys-lru." }))
        .await;

    let res = h.ok("search", json!({ "query": "postgres" })).await;
    assert_eq!(titles(&res), ["Postgres vacuum tuning", "Unrelated note"]);
    assert!(res["results"][0]["score"].as_f64().unwrap() > 0.0);
    h.finish().await;
}

#[tokio::test]
async fn search_filters_by_tags_kind_and_project() {
    let Some(h) = Harness::start().await else {
        return;
    };
    h.remember(json!({ "title": "A", "body": "x", "kind": "technique", "tags": ["rust", "ci"], "project": "p1" }))
        .await;
    h.remember(
        json!({ "title": "B", "body": "x", "kind": "memory", "tags": ["rust"], "project": "p1" }),
    )
    .await;
    h.remember(json!({ "title": "C", "body": "x", "kind": "technique", "tags": ["rust", "ci"], "project": "p2" }))
        .await;

    let all_tags = h.ok("search", json!({ "tags": ["RUST", "ci"] })).await;
    let mut t = titles(&all_tags);
    t.sort();
    assert_eq!(t, ["A", "C"], "tags must all match, case-insensitively");

    let memories = h.ok("search", json!({ "kind": "memory" })).await;
    assert_eq!(titles(&memories), ["B"]);

    let p2 = h.ok("search", json!({ "project": "p2" })).await;
    assert_eq!(titles(&p2), ["C"]);
    h.finish().await;
}

#[tokio::test]
async fn search_without_query_returns_newest_first_and_respects_limit() {
    let Some(h) = Harness::start().await else {
        return;
    };
    for title in ["first", "second", "third"] {
        h.remember(json!({ "title": title, "body": "x" })).await;
        // ObjectIds and timestamps have millisecond resolution; keep order unambiguous.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let res = h.ok("search", json!({ "limit": 2 })).await;
    assert_eq!(titles(&res), ["third", "second"]);

    // Out-of-range limits are clamped rather than rejected.
    let res = h.ok("search", json!({ "limit": 0 })).await;
    assert_eq!(titles(&res).len(), 1);
    h.finish().await;
}

#[tokio::test]
async fn search_snippets_are_truncated() {
    let Some(h) = Harness::start().await else {
        return;
    };
    h.remember(json!({ "title": "long", "body": "word ".repeat(200) }))
        .await;
    let res = h.ok("search", json!({ "query": "long" })).await;
    let snippet = res["results"][0]["snippet"].as_str().unwrap();
    assert!(snippet.ends_with('…'));
    assert!(snippet.chars().count() < 300);
    h.finish().await;
}

#[tokio::test]
async fn list_tags_counts_and_filters_by_project() {
    let Some(h) = Harness::start().await else {
        return;
    };
    h.remember(json!({ "title": "a", "body": "x", "tags": ["rust", "ci"], "project": "p1" }))
        .await;
    h.remember(json!({ "title": "b", "body": "x", "tags": ["rust"], "project": "p2" }))
        .await;

    let all = h.ok("list_tags", json!({})).await;
    assert_eq!(
        all["tags"],
        json!([{ "tag": "rust", "count": 2 }, { "tag": "ci", "count": 1 }])
    );
    let p2 = h.ok("list_tags", json!({ "project": "p2" })).await;
    assert_eq!(p2["tags"], json!([{ "tag": "rust", "count": 1 }]));
    h.finish().await;
}

#[tokio::test]
async fn invalid_input_is_rejected() {
    let Some(h) = Harness::start().await else {
        return;
    };
    assert!(h.call("get", json!({ "id": "not-an-id" })).await.is_err());
    assert!(
        h.call("remember", json!({ "title": "  ", "body": "x" }))
            .await
            .is_err()
    );
    assert!(
        h.call("remember", json!({ "title": "t", "body": "" }))
            .await
            .is_err()
    );
    let missing = ObjectId::new().to_hex();
    assert!(
        h.call("update", json!({ "id": missing, "title": "x" }))
            .await
            .is_err()
    );
    h.finish().await;
}

#[tokio::test]
async fn host_header_allow_list() {
    let Some(h) = Harness::start_with(HttpOptions {
        allowed_hosts: vec!["memory.example.ts.net".into()],
        allow_any_host: false,
    })
    .await
    else {
        return;
    };
    let http = reqwest::Client::new();
    let url = format!("http://{}/mcp", h.addr);
    let status_for = |host: &'static str| {
        let req = http
            .post(&url)
            .header("Host", host)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#);
        async move { req.send().await.unwrap().status().as_u16() }
    };
    assert_eq!(status_for("evil.example").await, 403);
    assert_eq!(status_for("memory.example.ts.net").await, 200);
    assert_eq!(status_for("localhost").await, 200);

    let health = http
        .get(format!("http://{}/healthz", h.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(health.text().await.unwrap(), "ok");
    h.finish().await;
}

const LONG_BODY: &str = "## Purpose\nShared memory.\n\n## Current status\n- 0.2.0 deployed\n- tests pass\n\n### Open items\n- backups\n\n## Decisions\n- no auth\n";

#[tokio::test]
async fn edit_replaces_text_and_appends_without_resending_the_body() {
    let Some(h) = Harness::start().await else {
        return;
    };
    let id = h
        .remember(json!({ "title": "State", "body": LONG_BODY }))
        .await;

    let saved = h
        .ok(
            "edit",
            json!({
                "id": id,
                "edits": [
                    { "old_text": "- 0.2.0 deployed", "new_text": "- 0.3.0 deployed" },
                    { "old_text": "- backups\n", "new_text": "" }
                ],
                "append": "## Related entries\n- none yet"
            }),
        )
        .await;
    assert_eq!(saved["replacements"], json!([1, 1]));
    assert!(saved.get("body").is_none());

    let body = h.ok("get", json!({ "id": id })).await["body"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(body.contains("- 0.3.0 deployed"));
    assert!(!body.contains("backups"));
    assert!(
        body.ends_with("- no auth\n\n## Related entries\n- none yet\n"),
        "{body}"
    );
    assert_eq!(saved["body_chars"], body.chars().count());

    h.finish().await;
}

#[tokio::test]
async fn failed_edits_change_nothing_and_explain_why() {
    let Some(h) = Harness::start().await else {
        return;
    };
    let id = h
        .remember(json!({ "title": "State", "body": LONG_BODY }))
        .await;

    let err = h
        .call(
            "edit",
            json!({ "id": id, "edits": [
                { "old_text": "Shared memory.", "new_text": "changed" },
                { "old_text": "not in the body", "new_text": "x" }
            ]}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("edits[1]") && err.contains("not found"),
        "{err}"
    );

    let err = h
        .call(
            "edit",
            json!({ "id": id, "edits": [{ "old_text": "- ", "new_text": "* " }] }),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("matches 4 places"), "{err}");

    assert_eq!(h.ok("get", json!({ "id": id })).await["body"], LONG_BODY);

    let all = h
        .ok(
            "edit",
            json!({ "id": id, "edits": [{ "old_text": "- ", "new_text": "* ", "replace_all": true }] }),
        )
        .await;
    assert_eq!(all["replacements"], json!([4]));

    let missing = ObjectId::new().to_hex();
    assert!(
        h.call("edit", json!({ "id": missing, "append": "x" }))
            .await
            .is_err()
    );
    assert!(h.call("edit", json!({ "id": id })).await.is_err());

    h.finish().await;
}

#[tokio::test]
async fn concurrent_edits_all_land() {
    let Some(h) = Harness::start().await else {
        return;
    };
    let id = h
        .remember(json!({ "title": "Log", "body": "# Log\n" }))
        .await;

    // Several sessions appending at once: with a blind read-modify-write
    // most of these would be lost.
    let calls = (0..8).map(|i| {
        h.call(
            "edit",
            json!({ "id": id, "append": format!("- entry {i}") }),
        )
    });
    for result in futures::future::join_all(calls).await {
        result.expect("edit");
    }

    let body = h.ok("get", json!({ "id": id })).await["body"]
        .as_str()
        .unwrap()
        .to_string();
    for i in 0..8 {
        assert!(
            body.contains(&format!("- entry {i}\n")),
            "entry {i} lost: {body}"
        );
    }

    h.finish().await;
}

#[tokio::test]
async fn get_returns_headings_and_single_sections() {
    let Some(h) = Harness::start().await else {
        return;
    };
    let id = h
        .remember(json!({ "title": "State", "body": LONG_BODY }))
        .await;

    let full = h.ok("get", json!({ "id": id })).await;
    assert_eq!(
        full["headings"],
        json!([
            "## Purpose",
            "## Current status",
            "### Open items",
            "## Decisions"
        ])
    );
    assert!(full.get("section").is_none());

    let part = h
        .ok("get", json!({ "id": id, "section": "current status" }))
        .await;
    assert_eq!(part["section"], "## Current status");
    assert_eq!(
        part["body"],
        "## Current status\n- 0.2.0 deployed\n- tests pass\n\n### Open items\n- backups\n\n"
    );

    let err = h
        .call("get", json!({ "id": id, "section": "nope" }))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("## Decisions"), "lists the headings: {err}");

    h.finish().await;
}

// ---------- web browser ----------

impl Harness {
    async fn page(&self, path: &str) -> (u16, String) {
        self.page_with_host(path, "localhost").await
    }

    async fn page_with_host(&self, path: &str, host: &str) -> (u16, String) {
        let resp = reqwest::Client::new()
            .get(format!("http://{}{path}", self.addr))
            .header("Host", host)
            .send()
            .await
            .unwrap();
        (resp.status().as_u16(), resp.text().await.unwrap())
    }
}

#[tokio::test]
async fn web_browses_projects_entries_and_search() {
    let Some(h) = Harness::start().await else {
        return;
    };
    let state = h
        .remember(json!({
            "title": "Project state: demo",
            "body": "## Purpose\nA demo.\n\n## Related entries\n- see the technique",
            "tags": ["project-state"],
            "project": "demo"
        }))
        .await;
    let technique = h
        .remember(json!({
            "title": "Flash the board",
            "body": "## Steps\n1. Hold **BOOT**\n2. Run `make flash`\n\nDetails in `000000000000000000000000`.",
            "kind": "technique",
            "tags": ["firmware"],
            "project": "demo"
        }))
        .await;
    h.remember(json!({ "title": "Global tip", "body": "Applies everywhere." }))
        .await;

    let (status, index) = h.page("/").await;
    assert_eq!(status, 200);
    assert!(index.contains(r#"href="/p/demo""#), "{index}");
    assert!(index.contains(r#"href="/general""#));
    assert!(index.contains("3 entries across 1 project."));

    let (status, project) = h.page("/p/demo").await;
    assert_eq!(status, 200);
    assert!(project.contains("Start here"));
    assert!(project.contains(&format!(r#"href="/m/{state}""#)));
    assert!(project.contains(&format!(r#"href="/m/{technique}""#)));
    assert!(!project.contains("Global tip"));

    let (_, general) = h.page("/general").await;
    assert!(general.contains("Global tip") && !general.contains("Flash the board"));

    let (status, entry) = h.page(&format!("/m/{technique}")).await;
    assert_eq!(status, 200);
    assert!(entry.contains("<h1>Flash the board</h1>"));
    assert!(entry.contains(r#"<h2 id="steps">Steps</h2>"#));
    assert!(entry.contains("<strong>BOOT</strong>"));
    assert!(
        entry.contains(r#"href="/m/000000000000000000000000""#),
        "entry ids link"
    );

    let (status, results) = h.page("/search?q=flash").await;
    assert_eq!(status, 200);
    assert!(results.contains("1 result for"));

    assert_eq!(h.page("/p/nothing-here").await.0, 404);
    assert_eq!(h.page("/m/not-an-id").await.0, 404);
    assert_eq!(h.page(&format!("/m/{}", ObjectId::new())).await.0, 404);
    assert_eq!(h.page("/static/style.css").await.0, 200);
    h.finish().await;
}

#[tokio::test]
async fn web_escapes_agent_written_html() {
    let Some(h) = Harness::start().await else {
        return;
    };
    let id = h
        .remember(json!({
            "title": "<img src=x onerror=alert(1)>",
            "body": "<script>alert(1)</script>\n\n[click](javascript:alert(1))",
            "project": "<b>p</b>"
        }))
        .await;
    let (_, page) = h.page(&format!("/m/{id}")).await;
    assert!(!page.contains("<script>alert"), "{page}");
    assert!(!page.contains("<img src=x"), "{page}");
    assert!(!page.contains("javascript:"), "{page}");
    assert!(page.contains("&lt;script&gt;"));
    let (_, index) = h.page("/").await;
    assert!(!index.contains("<b>p</b>") && index.contains("/p/%3Cb%3Ep%3C%2Fb%3E"));
    h.finish().await;
}

#[tokio::test]
async fn web_uses_the_host_allow_list() {
    let Some(h) = Harness::start_with(HttpOptions {
        allowed_hosts: vec!["memory.example.ts.net".into()],
        allow_any_host: false,
    })
    .await
    else {
        return;
    };
    assert_eq!(h.page_with_host("/", "evil.example").await.0, 403);
    assert_eq!(h.page_with_host("/", "evil.example:8787").await.0, 403);
    assert_eq!(
        h.page_with_host("/", "memory.example.ts.net:8787").await.0,
        200
    );
    assert_eq!(h.page_with_host("/", "localhost").await.0, 200);
    h.finish().await;
}
