//! What the mod asks T3 Code, a port of `t3/T3Api.kt`: environments, projects,
//! open threads, one thread's messages and a reply, over the MCP tools.

use crate::mcp::{McpClient, McpError};
use crate::model::*;
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};

const MAX_CHARS_PER_ITEM: i64 = 20_000;

pub struct T3Api<'a> {
    mcp: &'a mut McpClient,
}

impl<'a> T3Api<'a> {
    pub fn new(mcp: &'a mut McpClient) -> Self {
        Self { mcp }
    }

    pub fn local_environment(&mut self) -> Result<Environment, McpError> {
        let result = self.mcp.call_tool("t3_environment_read", json!({}))?;
        let label = str_of(&result, "label");
        Ok(Environment {
            id: None,
            label: if label.is_empty() {
                "This machine".to_owned()
            } else {
                label
            },
        })
    }

    pub fn reachable_linked_environments(&mut self) -> Vec<Environment> {
        let result = self.mcp.call_tool("t3_environment_links", json!({}));
        let Ok(result) = result else {
            return Vec::new();
        };
        objects(result.get("links"))
            .filter(|link| str_of(link, "status") == "reachable")
            .map(|link| Environment {
                id: opt_str(link.get("environmentId")),
                label: str_of(&link, "label"),
            })
            .collect()
    }

    pub fn list_projects(&mut self, environment: &Environment) -> Result<Vec<Project>, McpError> {
        paged(|cursor| {
            let page = self.mcp.call_tool(
                "t3_project_list",
                args(&[
                    ("limit", json!(100)),
                    ("cursor", json!(cursor)),
                    ("environmentId", json!(environment.id)),
                ]),
            )?;
            let projects = objects(page.get("projects"))
                .map(|p| {
                    Project {
                        id: str_of(&p, "id"),
                        title: str_of(&p, "title"),
                        environment: environment.clone(),
                    }
                })
                .collect();
            Ok((projects, next_cursor(&page)))
        })
    }

    pub fn list_open_threads(&mut self, project: &Project) -> Result<Vec<ThreadSummary>, McpError> {
        paged(|cursor| {
            let page = self.mcp.call_tool(
                "t3_thread_list",
                args(&[
                    ("projectId", json!(project.id)),
                    ("environmentId", json!(project.environment.id)),
                    ("includeSubagents", json!(false)),
                    ("settled", json!(false)),
                    ("snoozed", json!(false)),
                    ("limit", json!(100)),
                    ("cursor", json!(cursor)),
                ]),
            )?;
            let threads = objects(page.get("threads"))
                .map(|t| Self::to_summary(&t, project))
                .collect();
            Ok((threads, next_cursor(&page)))
        })
    }

    /// The thread's last `wanted` messages, paging back until enough or the
    /// start of the thread — the Java mod's growing-span walk.
    pub fn read_thread(
        &mut self,
        thread_ref: &ThreadRef,
        wanted: usize,
    ) -> Result<ThreadDetail, McpError> {
        let head = self.read_page(thread_ref, None, 1)?;
        let item_count = int_of(&head["thread"], "itemCount");
        let mut span = wanted * 4;
        let (mut messages, mut latest);
        loop {
            let start = (item_count as usize).saturating_sub(span);
            let (collected, last) = self.read_messages_from(thread_ref, start)?;
            messages = collected;
            latest = last;
            if messages.len() >= wanted || start == 0 {
                break;
            }
            span *= 2;
        }

        let thread = &latest["thread"];
        Ok(ThreadDetail {
            thread_ref: thread_ref.clone(),
            title: str_of(thread, "title"),
            status: str_of(thread, "status"),
            pending_request_count: int_of(thread, "pendingRequestCount"),
            model: str_of(thread, "model"),
            runtime_mode: str_of(thread, "runtimeMode"),
            branch: str_of(thread, "branch"),
            latest_run: Self::latest_run_timing_of(&head),
            messages: messages.split_off(messages.len().saturating_sub(wanted)),
        })
    }

    pub fn latest_run_timing(
        &mut self,
        thread_ref: &ThreadRef,
    ) -> Result<Option<RunTiming>, McpError> {
        let page = self.read_page(thread_ref, None, 1)?;
        Ok(Self::latest_run_timing_of(&page))
    }

    /// `recentRuns`' timing for the page's `latestRunId` (or its first run).
    fn latest_run_timing_of(page: &Value) -> Option<RunTiming> {
        let latest_run_id = str_of(&page["thread"], "latestRunId");
        let runs: Vec<Value> = objects(page.get("recentRuns")).collect();
        let run = runs
            .iter()
            .find(|r| str_of(r, "runId") == latest_run_id)
            .or_else(|| runs.first())?;
        Some(RunTiming {
            started_at: instant_of(run, "startedAt").or_else(|| instant_of(run, "requestedAt")),
            completed_at: instant_of(run, "completedAt"),
        })
    }

    pub fn send_message(&mut self, thread_ref: &ThreadRef, message: &str) -> Result<(), McpError> {
        self.mcp.call_tool(
            "t3_thread_send",
            args(&[
                ("threadId", json!(thread_ref.thread_id)),
                ("environmentId", json!(thread_ref.environment_id)),
                ("message", json!(message)),
                ("mode", json!("queue")),
            ]),
        )?;
        Ok(())
    }

    fn read_messages_from(
        &mut self,
        thread_ref: &ThreadRef,
        start: usize,
    ) -> Result<(Vec<ThreadMessage>, Value), McpError> {
        let mut messages = Vec::new();
        let mut after_position = (start > 0).then_some(start as i64);
        let page;
        loop {
            let current = self.read_page(thread_ref, after_position, 100)?;
            for item in objects(current.get("items")) {
                if let Some(message) = self.to_message(thread_ref, &item) {
                    messages.push(message);
                }
            }
            after_position = current.get("nextPosition").and_then(Value::as_i64);
            if !(current.get("hasMore").and_then(Value::as_bool) == Some(true)
                && after_position.is_some())
            {
                page = current;
                break;
            }
        }
        Ok((messages, page))
    }

    fn read_page(
        &mut self,
        thread_ref: &ThreadRef,
        after_position: Option<i64>,
        limit: i64,
    ) -> Result<Value, McpError> {
        self.mcp.call_tool(
            "t3_thread_read",
            args(&[
                ("threadId", json!(thread_ref.thread_id)),
                ("environmentId", json!(thread_ref.environment_id)),
                ("view", json!("messages")),
                ("limit", json!(limit)),
                ("runLimit", json!(1)),
                ("maxCharsPerItem", json!(MAX_CHARS_PER_ITEM)),
                ("afterPosition", json!(after_position)),
            ]),
        )
    }

    fn to_message(&mut self, thread_ref: &ThreadRef, item: &Value) -> Option<ThreadMessage> {
        let role = match str_of(item, "type").as_str() {
            "user_message" => MessageRole::User,
            "assistant_message" => MessageRole::Assistant,
            _ => return None,
        };
        Some(ThreadMessage {
            role,
            text: self.full_text(thread_ref, item),
        })
    }

    /// An item's text, following `textTruncated`/`nextTextOffset` for the rest.
    fn full_text(&mut self, thread_ref: &ThreadRef, item: &Value) -> String {
        let mut text = str_of(item, "text");
        let mut offset = next_text_offset(item);
        while let Some(position) = offset {
            let Ok(page) = self.mcp.call_tool(
                "t3_thread_read",
                args(&[
                    ("threadId", json!(thread_ref.thread_id)),
                    ("environmentId", json!(thread_ref.environment_id)),
                    ("itemId", json!(str_of(item, "itemId"))),
                    ("textOffset", json!(position)),
                    ("runLimit", json!(1)),
                ]),
            ) else {
                break;
            };
            let Some(chunk) = objects(page.get("items"))
                .find(|i| str_of(i, "itemId") == str_of(item, "itemId"))
            else {
                break;
            };
            text.push_str(&str_of(&chunk, "text"));
            offset = next_text_offset(&chunk);
        }
        text
    }

    fn to_summary(thread: &Value, project: &Project) -> ThreadSummary {
        let latest_run_id = str_of(thread, "latestRunId");
        ThreadSummary {
            thread_ref: ThreadRef {
                environment_id: project.environment.id.clone(),
                thread_id: str_of(thread, "threadId"),
            },
            project_title: project.title.clone(),
            environment: project.environment.clone(),
            title: str_of(thread, "title"),
            status: str_of(thread, "status"),
            updated_at: instant_of(thread, "updatedAt"),
            model: str_of(thread, "model"),
            runtime_mode: str_of(thread, "runtimeMode"),
            latest_run_id: (!latest_run_id.is_empty()).then_some(latest_run_id),
            run_started_at: None,
        }
    }
}

// ---- JSON access, mirroring the Kotlin file's little helpers. ----

pub fn str_of(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn opt_str(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

fn int_of(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn instant_of(value: &Value, key: &str) -> Option<DateTime<Utc>> {
    let text = str_of(value, key);
    if text.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(&text)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn objects(value: Option<&Value>) -> impl Iterator<Item = Value> {
    value
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
}

fn next_cursor(page: &Value) -> Option<i64> {
    page.get("nextCursor").and_then(Value::as_i64)
}

fn next_text_offset(item: &Value) -> Option<i64> {
    if item.get("textTruncated").and_then(Value::as_bool) == Some(true) {
        item.get("nextTextOffset").and_then(Value::as_i64)
    } else {
        None
    }
}

/// An arguments object: nulls are left out, as the Kotlin `args` builder does.
fn args(pairs: &[(&str, Value)]) -> Value {
    let mut map = Map::new();
    for (key, value) in pairs {
        if !value.is_null() {
            map.insert((*key).to_owned(), value.clone());
        }
    }
    Value::Object(map)
}

/// Fetch pages until there is no next cursor.
fn paged<T>(
    mut fetch: impl FnMut(Option<i64>) -> Result<(Vec<T>, Option<i64>), McpError>,
) -> Result<Vec<T>, McpError> {
    let mut all = Vec::new();
    let mut cursor = Some(0);
    while let Some(at) = cursor {
        let (items, next) = fetch(Some(at))?;
        all.extend(items);
        cursor = next;
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn args_omits_nulls() {
        let value = args(&[("a", json!(1)), ("b", json!(null)), ("c", json!("x"))]);
        assert_eq!(value, json!({"a": 1, "c": "x"}));
    }

    #[test]
    fn summary_from_a_thread_object() {
        let project = Project {
            id: "p".into(),
            title: "Proj".into(),
            environment: Environment {
                id: None,
                label: "here".into(),
            },
        };
        let summary = T3Api::to_summary(
            &json!({
                "threadId": "t1",
                "title": "Fix it",
                "status": "waiting",
                "updatedAt": "2026-11-09T08:00:00Z",
                "latestRunId": "r9",
            }),
            &project,
        );
        assert_eq!(summary.thread_ref.thread_id, "t1");
        assert_eq!(summary.attention(), Attention::NeedsInput);
        assert!(summary.updated_at.is_some());
        assert_eq!(summary.latest_run_id.as_deref(), Some("r9"));
    }
}
