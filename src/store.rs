//! MongoDB-backed storage for shared agent memories.

use anyhow::{Context, Result};
use futures::TryStreamExt;
use mongodb::{
    Client, Collection, IndexModel,
    bson::{self, Document, doc, oid::ObjectId},
    options::{IndexOptions, ReturnDocument},
};
use serde::{Deserialize, Serialize};

pub const COLLECTION: &str = "memories";

/// What kind of knowledge an entry holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A fact, decision, or piece of context worth remembering.
    Memory,
    /// A reusable way of doing something: a recipe, workflow, or trick.
    Technique,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Memory => "memory",
            Kind::Technique => "technique",
        }
    }
}

/// A memory as stored in MongoDB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDoc {
    #[serde(rename = "_id")]
    pub id: ObjectId,
    pub title: String,
    pub body: String,
    pub kind: Kind,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
    /// Text-search relevance; only present on search results, never stored.
    #[serde(default, skip_serializing)]
    pub score: Option<f64>,
}

pub struct NewMemory {
    pub title: String,
    pub body: String,
    pub kind: Kind,
    pub tags: Vec<String>,
    pub project: Option<String>,
    pub author: Option<String>,
}

#[derive(Default)]
pub struct MemoryPatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub kind: Option<Kind>,
    pub tags: Option<Vec<String>>,
    pub project: Option<String>,
}

#[derive(Default)]
pub struct SearchQuery {
    pub text: Option<String>,
    pub tags: Vec<String>,
    pub kind: Option<Kind>,
    pub project: Option<String>,
    pub limit: i64,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TagCount {
    pub tag: String,
    pub count: i64,
}

#[derive(Clone)]
pub struct Store {
    memories: Collection<MemoryDoc>,
}

/// Lowercase, trim, drop empties and duplicates so tag filters match reliably.
pub fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = tags
        .into_iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

impl Store {
    pub async fn connect(uri: &str, database: &str) -> Result<Self> {
        let client = Client::with_uri_str(uri)
            .await
            .with_context(|| format!("parsing MongoDB URI {uri}"))?;
        let db = client.database(database);
        db.run_command(doc! { "ping": 1 })
            .await
            .with_context(|| format!("pinging MongoDB at {uri}"))?;
        let store = Self {
            memories: db.collection(COLLECTION),
        };
        store.ensure_indexes().await?;
        Ok(store)
    }

    async fn ensure_indexes(&self) -> Result<()> {
        let text = IndexModel::builder()
            .keys(doc! { "title": "text", "tags": "text", "body": "text" })
            .options(
                IndexOptions::builder()
                    .name("memories_text".to_string())
                    .weights(doc! { "title": 10, "tags": 5, "body": 1 })
                    .build(),
            )
            .build();
        let tags = IndexModel::builder().keys(doc! { "tags": 1 }).build();
        let recent = IndexModel::builder()
            .keys(doc! { "project": 1, "updated_at": -1 })
            .build();
        self.memories
            .create_indexes([text, tags, recent])
            .await
            .context("creating indexes")?;
        Ok(())
    }

    pub async fn insert(&self, new: NewMemory) -> Result<MemoryDoc> {
        let now = bson::DateTime::now();
        let memory = MemoryDoc {
            id: ObjectId::new(),
            title: new.title,
            body: new.body,
            kind: new.kind,
            tags: normalize_tags(new.tags),
            project: new.project,
            author: new.author,
            created_at: now,
            updated_at: now,
            score: None,
        };
        self.memories.insert_one(&memory).await?;
        Ok(memory)
    }

    pub async fn get(&self, id: ObjectId) -> Result<Option<MemoryDoc>> {
        Ok(self.memories.find_one(doc! { "_id": id }).await?)
    }

    pub async fn update(&self, id: ObjectId, patch: MemoryPatch) -> Result<Option<MemoryDoc>> {
        let mut set = doc! { "updated_at": bson::DateTime::now() };
        if let Some(title) = patch.title {
            set.insert("title", title);
        }
        if let Some(body) = patch.body {
            set.insert("body", body);
        }
        if let Some(kind) = patch.kind {
            set.insert("kind", kind.as_str());
        }
        if let Some(tags) = patch.tags {
            set.insert("tags", normalize_tags(tags));
        }
        if let Some(project) = patch.project {
            set.insert("project", project);
        }
        Ok(self
            .memories
            .find_one_and_update(doc! { "_id": id }, doc! { "$set": set })
            .return_document(ReturnDocument::After)
            .await?)
    }

    pub async fn delete(&self, id: ObjectId) -> Result<bool> {
        Ok(self
            .memories
            .delete_one(doc! { "_id": id })
            .await?
            .deleted_count
            == 1)
    }

    pub async fn search(&self, q: SearchQuery) -> Result<Vec<MemoryDoc>> {
        let mut filter = Document::new();
        let text = q.text.as_deref().map(str::trim).filter(|t| !t.is_empty());
        if let Some(text) = text {
            filter.insert("$text", doc! { "$search": text });
        }
        let tags = normalize_tags(q.tags);
        if !tags.is_empty() {
            filter.insert("tags", doc! { "$all": tags });
        }
        if let Some(kind) = q.kind {
            filter.insert("kind", kind.as_str());
        }
        if let Some(project) = q.project {
            filter.insert("project", project);
        }

        let find = self.memories.find(filter).limit(q.limit);
        let find = if text.is_some() {
            find.projection(doc! { "score": { "$meta": "textScore" } })
                .sort(doc! { "score": { "$meta": "textScore" }, "updated_at": -1 })
        } else {
            find.sort(doc! { "updated_at": -1 })
        };
        Ok(find.await?.try_collect().await?)
    }

    pub async fn tag_counts(&self, project: Option<String>) -> Result<Vec<TagCount>> {
        let mut pipeline = Vec::new();
        if let Some(project) = project {
            pipeline.push(doc! { "$match": { "project": project } });
        }
        pipeline.extend([
            doc! { "$unwind": "$tags" },
            doc! { "$group": { "_id": "$tags", "count": { "$sum": 1 } } },
            doc! { "$sort": { "count": -1, "_id": 1 } },
        ]);
        let docs: Vec<Document> = self
            .memories
            .aggregate(pipeline)
            .await?
            .try_collect()
            .await?;
        Ok(docs
            .into_iter()
            .filter_map(|d| {
                Some(TagCount {
                    tag: d.get_str("_id").ok()?.to_string(),
                    count: d
                        .get_i32("count")
                        .map(i64::from)
                        .or_else(|_| d.get_i64("count"))
                        .ok()?,
                })
            })
            .collect())
    }
}
