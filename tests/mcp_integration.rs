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
        ["forget", "get", "list_tags", "remember", "search", "update"]
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
    assert_eq!(saved["created_at"], saved["updated_at"]);

    let got = h.ok("get", json!({ "id": id })).await;
    assert_eq!(got, saved);

    let updated = h
        .ok(
            "update",
            json!({ "id": id, "tags": ["cargo", "rust"], "project": "demo" }),
        )
        .await;
    assert_eq!(updated["tags"], json!(["cargo", "rust"]));
    assert_eq!(updated["project"], "demo");
    assert_eq!(
        updated["body"], saved["body"],
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
