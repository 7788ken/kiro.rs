//! Prompt cache 用量模拟
//!
//! 上游（Kiro）不返回 prompt cache 数据。这里按 Anthropic 官方规则在本地模拟
//! `cache_creation_input_tokens` / `cache_read_input_tokens`，只用于展示和记录，不影响扣费。
//!
//! 规则：
//! - 断点按 `tools` → `system` → `messages`（逐 content block）顺序扫描，带 `cache_control` 的位置记为断点，只取最后 4 个；
//! - 断点键 = 该断点及之前所有段落内容的稳定哈希（哈希前去掉 `cache_control` 本身，断点移动不影响前缀）；
//! - 缓存表 TTL 5 分钟，只写入断点；查找时从最后一个断点及其之前最多 20 个 block 边界、
//!   再加更早的断点里，按前缀从长到短找第一个未过期的命中：命中前缀记为读取，
//!   命中位置到最后断点之间记为写入，其余记为普通输入。

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Value, json};

use super::types::MessagesRequest;
use crate::token;

/// 缓存条目有效期（与官方默认 5 分钟一致）
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
/// 每个请求最多生效的断点数
const MAX_BREAKPOINTS: usize = 4;
/// 查找命中时从最后断点往前回看的 block 边界数（与官方约 20 个一致）
const LOOKBACK_BOUNDARIES: usize = 20;
/// 缓存表超过此条数时，插入前先清理过期项
const MAX_ENTRIES_BEFORE_PRUNE: usize = 10_000;

/// 本地口径的缓存拆分结果（token 数按 `token::count_tokens` 估算）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheSplit {
    total_local: i64,
    read_local: i64,
    creation_local: i64,
}

/// 返回给请求方的 usage token 拆分
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageTokens {
    pub input_tokens: i32,
    pub cache_creation_input_tokens: i32,
    pub cache_read_input_tokens: i32,
}

impl CacheSplit {
    /// 无断点：全部计为普通输入
    pub fn none(total_local: i64) -> Self {
        Self {
            total_local: total_local.max(0),
            read_local: 0,
            creation_local: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_parts(total_local: i64, read_local: i64, creation_local: i64) -> Self {
        Self {
            total_local,
            read_local,
            creation_local,
        }
    }

    /// 按配置的口径选定上报总量，再做拆分。
    ///
    /// `upstream_total` 是上游上下文占用百分比换算出的值，含整段会话的累计占用；
    /// `Local` 与 `Min` 用本地估算把它压回本次请求的量级。
    pub fn apply_with_source(
        &self,
        upstream_total: i32,
        source: crate::model::config::InputTokenSource,
    ) -> UsageTokens {
        use crate::model::config::InputTokenSource;
        let local = self.total_local.clamp(0, i64::from(i32::MAX)) as i32;
        let total = match source {
            InputTokenSource::Upstream => upstream_total,
            // 本地估算为 0（例如空请求）时没有可用口径，退回上游值
            InputTokenSource::Local if local > 0 => local,
            InputTokenSource::Local => upstream_total,
            InputTokenSource::Min if local > 0 => upstream_total.min(local),
            InputTokenSource::Min => upstream_total,
        };
        self.apply(total)
    }

    /// 把本地口径按比例缩放到真实总量 `actual_total`。
    ///
    /// 三者之和 == `actual_total.max(1)`，且 `input_tokens >= 1`
    /// （不足时先从 cache_read、再从 cache_creation 里让出来）。
    pub fn apply(&self, actual_total: i32) -> UsageTokens {
        let total = i64::from(actual_total.max(1));
        let (mut read, mut creation) = if self.total_local > 0 {
            (
                self.read_local.clamp(0, self.total_local) * total / self.total_local,
                self.creation_local.clamp(0, self.total_local) * total / self.total_local,
            )
        } else {
            (0, 0)
        };
        if read + creation > total {
            // 防御：本地口径异常时把超出部分从写入里扣掉
            creation = (total - read).max(0);
            read = read.min(total);
        }
        if total - read - creation < 1 {
            if read > 0 {
                read -= 1;
            } else if creation > 0 {
                creation -= 1;
            }
        }
        let input = total - read - creation;
        UsageTokens {
            input_tokens: input as i32,
            cache_creation_input_tokens: creation as i32,
            cache_read_input_tokens: read as i32,
        }
    }
}

impl UsageTokens {
    /// 生成 Anthropic 格式的 usage 对象
    pub fn to_usage_json(&self, output_tokens: i32) -> Value {
        json!({
            "input_tokens": self.input_tokens,
            "cache_creation_input_tokens": self.cache_creation_input_tokens,
            "cache_read_input_tokens": self.cache_read_input_tokens,
            "output_tokens": output_tokens
        })
    }
}

/// 模拟本次请求的缓存命中情况（全局缓存表，有副作用：写入/刷新断点）
pub fn simulate(payload: &MessagesRequest) -> CacheSplit {
    static CACHE: LazyLock<PromptCache> = LazyLock::new(PromptCache::new);
    CACHE.simulate_at(payload, Instant::now())
}

/// 按配置模拟缓存：`Disabled` 不读写全局缓存表，输入全部计为普通输入
pub fn simulate_with_mode(
    payload: &MessagesRequest,
    mode: crate::model::config::PromptCacheMode,
) -> CacheSplit {
    use crate::model::config::PromptCacheMode;
    match mode {
        PromptCacheMode::Simulated => simulate(payload),
        PromptCacheMode::Disabled => CacheSplit::none(token::count_all_tokens_local(
            payload.system.clone(),
            payload.messages.clone(),
            payload.tools.clone(),
        ) as i64),
    }
}

/// 一个 block 边界：前缀哈希 + 前缀本地 token 数 + 是否为断点（带 `cache_control`）
#[derive(Debug, Clone, Copy)]
struct Boundary {
    key: u64,
    prefix_tokens: i64,
    is_breakpoint: bool,
}

struct PromptCache {
    entries: Mutex<HashMap<u64, Instant>>,
}

impl PromptCache {
    fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn simulate_at(&self, payload: &MessagesRequest, now: Instant) -> CacheSplit {
        let (boundaries, total_local) = scan_breakpoints(payload);
        // 断点在 boundaries 里的下标，只取最后 MAX_BREAKPOINTS 个
        let mut breakpoint_indices: Vec<usize> = boundaries
            .iter()
            .enumerate()
            .filter(|(_, b)| b.is_breakpoint)
            .map(|(i, _)| i)
            .collect();
        if breakpoint_indices.is_empty() {
            return CacheSplit::none(total_local);
        }
        if breakpoint_indices.len() > MAX_BREAKPOINTS {
            breakpoint_indices.drain(..breakpoint_indices.len() - MAX_BREAKPOINTS);
        }
        let last_index = *breakpoint_indices.last().unwrap();
        let last_prefix = boundaries[last_index].prefix_tokens;

        // 候选命中位置：最后断点及其前 LOOKBACK_BOUNDARIES 个边界，再加更早的断点。
        // 前缀 token 数随位置单调不减，按位置倒序即按 prefix_tokens 从大到小。
        let mut candidates: Vec<usize> =
            (last_index.saturating_sub(LOOKBACK_BOUNDARIES)..=last_index).collect();
        candidates.extend(breakpoint_indices.iter().copied());
        candidates.sort_unstable_by(|a, b| b.cmp(a));
        candidates.dedup();

        let mut entries = self.entries.lock();
        let read_local = candidates
            .iter()
            .map(|&i| &boundaries[i])
            .find(|b| {
                entries
                    .get(&b.key)
                    .is_some_and(|at| now.saturating_duration_since(*at) < CACHE_TTL)
            })
            .map(|b| b.prefix_tokens)
            .unwrap_or(0);

        if entries.len() > MAX_ENTRIES_BEFORE_PRUNE {
            entries.retain(|_, at| now.saturating_duration_since(*at) < CACHE_TTL);
        }
        for &i in &breakpoint_indices {
            entries.insert(boundaries[i].key, now);
        }

        CacheSplit {
            total_local,
            read_local,
            creation_local: (last_prefix - read_local).max(0),
        }
    }
}

/// 按官方顺序扫描，返回（每个 block 边界，本地口径总 token 数）。
///
/// tools 每个 tool、system 每个 block、messages 每个 block（字符串消息整条算一个）各是一个边界。
fn scan_breakpoints(payload: &MessagesRequest) -> (Vec<Boundary>, i64) {
    let mut hasher = DefaultHasher::new();
    let mut tokens: i64 = 0;
    let mut boundaries = Vec::new();
    let mut push_boundary = |hasher: &DefaultHasher, tokens: i64, is_breakpoint: bool| {
        boundaries.push(Boundary {
            key: hasher.clone().finish(),
            prefix_tokens: tokens,
            is_breakpoint,
        });
    };

    if let Some(tools) = &payload.tools {
        for tool in tools {
            "tool".hash(&mut hasher);
            let value = serde_json::to_value(tool).unwrap_or(Value::Null);
            stable_json(value).hash(&mut hasher);
            tokens += count(&tool.name) + count(&tool.description);
            tokens += count(&serde_json::to_string(&tool.input_schema).unwrap_or_default());
            push_boundary(
                &hasher,
                tokens,
                has_cache_control(tool.cache_control.as_ref()),
            );
        }
    }

    if let Some(system) = &payload.system {
        for block in system {
            "system".hash(&mut hasher);
            let value = serde_json::to_value(block).unwrap_or(Value::Null);
            stable_json(value).hash(&mut hasher);
            tokens += count(&block.text);
            push_boundary(
                &hasher,
                tokens,
                has_cache_control(block.cache_control.as_ref()),
            );
        }
    }

    for message in &payload.messages {
        match &message.content {
            Value::Array(blocks) => {
                for block in blocks {
                    "message".hash(&mut hasher);
                    message.role.hash(&mut hasher);
                    stable_json(block.clone()).hash(&mut hasher);
                    tokens += block_tokens(block);
                    push_boundary(
                        &hasher,
                        tokens,
                        has_cache_control(block.get("cache_control")),
                    );
                }
            }
            other => {
                "message".hash(&mut hasher);
                message.role.hash(&mut hasher);
                stable_json(other.clone()).hash(&mut hasher);
                if let Value::String(text) = other {
                    tokens += count(text);
                }
                push_boundary(&hasher, tokens, false);
            }
        }
    }

    (boundaries, tokens)
}

fn count(text: &str) -> i64 {
    token::count_tokens(text) as i64
}

/// 图片 / 文档 block 的固定估算值（官方图片约 1600 token 上限的近似；文档无法估算，同样取此值）
const MEDIA_BLOCK_TOKENS: i64 = 1500;

/// 估算一个消息 content block 的本地 token 数（按 `type` 分支，未知类型或缺字段计 0）
fn block_tokens(block: &Value) -> i64 {
    let str_field = |key: &str| block.get(key).and_then(Value::as_str).map_or(0, count);
    match block.get("type").and_then(Value::as_str) {
        Some("text") => str_field("text"),
        Some("thinking") => str_field("thinking"),
        Some("redacted_thinking") => str_field("data"),
        Some("tool_use") => {
            let input = block
                .get("input")
                .map_or(0, |input| count(&input.to_string()));
            str_field("name") + input
        }
        Some("tool_result") => match block.get("content") {
            Some(Value::String(text)) => count(text),
            Some(Value::Array(blocks)) => blocks.iter().map(block_tokens).sum(),
            _ => 0,
        },
        Some("image") | Some("document") => MEDIA_BLOCK_TOKENS,
        _ => 0,
    }
}

fn has_cache_control(value: Option<&Value>) -> bool {
    value.is_some_and(|v| !v.is_null())
}

/// 去掉顶层 `cache_control` 后序列化（serde_json 未开 preserve_order，对象键有序，输出稳定）
fn stable_json(mut value: Value) -> String {
    if let Value::Object(map) = &mut value {
        map.remove("cache_control");
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(value: Value) -> MessagesRequest {
        serde_json::from_value(value).unwrap()
    }

    fn long_text(seed: &str) -> String {
        format!("{seed} ").repeat(200)
    }

    fn base_request(with_extra_message: bool) -> MessagesRequest {
        let mut messages = vec![json!({
            "role": "user",
            "content": [{
                "type": "text",
                "text": long_text("first question"),
                "cache_control": {"type": "ephemeral"}
            }]
        })];
        if with_extra_message {
            messages.push(json!({
                "role": "assistant",
                "content": [{"type": "text", "text": long_text("an answer")}]
            }));
            messages.push(json!({
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": long_text("follow up"),
                    "cache_control": {"type": "ephemeral"}
                }]
            }));
        }
        request(json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "tools": [{
                "name": "read_file",
                "description": long_text("reads a file"),
                "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}},
                "cache_control": {"type": "ephemeral"}
            }],
            "system": [{
                "type": "text",
                "text": long_text("you are helpful"),
                "cache_control": {"type": "ephemeral"}
            }],
            "messages": messages
        }))
    }

    fn local_total(payload: &MessagesRequest) -> i64 {
        token::count_all_tokens_local(
            payload.system.clone(),
            payload.messages.clone(),
            payload.tools.clone(),
        ) as i64
    }

    #[test]
    fn cache_control_survives_deserialization() {
        let payload = base_request(false);
        assert!(payload.tools.as_ref().unwrap()[0].cache_control.is_some());
        assert!(payload.system.as_ref().unwrap()[0].cache_control.is_some());
    }

    #[test]
    fn no_breakpoint_is_all_input() {
        let cache = PromptCache::new();
        let payload = request(json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "system": "plain system prompt",
            "messages": [{"role": "user", "content": "hello there"}]
        }));
        let split = cache.simulate_at(&payload, Instant::now());
        assert_eq!(split, CacheSplit::none(local_total(&payload)));
        assert_eq!(split.read_local, 0);
        assert_eq!(split.creation_local, 0);
        assert!(cache.entries.lock().is_empty());
        let usage = split.apply(500);
        assert_eq!(usage.input_tokens, 500);
        assert_eq!(usage.cache_creation_input_tokens, 0);
        assert_eq!(usage.cache_read_input_tokens, 0);
    }

    #[test]
    fn input_token_source_picks_reported_total() {
        use crate::model::config::InputTokenSource;
        // 本地估算 1000，上游换算值 50000（含整段会话累计占用）
        let split = CacheSplit::from_parts(1000, 600, 300);

        let upstream = split.apply_with_source(50_000, InputTokenSource::Upstream);
        let upstream_sum = upstream.input_tokens
            + upstream.cache_creation_input_tokens
            + upstream.cache_read_input_tokens;
        assert_eq!(upstream_sum, 50_000);

        let local = split.apply_with_source(50_000, InputTokenSource::Local);
        let local_sum =
            local.input_tokens + local.cache_creation_input_tokens + local.cache_read_input_tokens;
        assert_eq!(local_sum, 1000, "Local 用本地估算压回本次请求量级");

        let min = split.apply_with_source(50_000, InputTokenSource::Min);
        let min_sum =
            min.input_tokens + min.cache_creation_input_tokens + min.cache_read_input_tokens;
        assert_eq!(min_sum, 1000, "Min 取较小者");

        // 上游值更小时 Min 取上游
        let min_small = split.apply_with_source(400, InputTokenSource::Min);
        let min_small_sum = min_small.input_tokens
            + min_small.cache_creation_input_tokens
            + min_small.cache_read_input_tokens;
        assert_eq!(min_small_sum, 400);

        // 本地估算为 0 时退回上游值，避免上报 0
        let empty = CacheSplit::none(0);
        for source in [InputTokenSource::Local, InputTokenSource::Min] {
            let usage = empty.apply_with_source(777, source);
            assert_eq!(usage.input_tokens, 777, "{source:?} 无本地口径时退回上游");
        }
    }

    #[test]
    fn apply_with_source_keeps_split_invariants() {
        use crate::model::config::InputTokenSource;
        // 本地口径取多种比例，上游总量从远大到远小，覆盖 Min/Local 缩小后的拆分
        for (total_local, read, creation) in [
            (1000i64, 600i64, 300i64),
            (1000, 1000, 0),
            (1000, 0, 1000),
            (1000, 999, 1),
            (1, 1, 0),
            (0, 0, 0),
        ] {
            let split = CacheSplit::from_parts(total_local, read, creation);
            for upstream in [1i32, 2, 7, 500, 50_000, i32::MAX / 2] {
                for source in [
                    InputTokenSource::Upstream,
                    InputTokenSource::Local,
                    InputTokenSource::Min,
                ] {
                    let u = split.apply_with_source(upstream, source);
                    let sum = i64::from(u.input_tokens)
                        + i64::from(u.cache_creation_input_tokens)
                        + i64::from(u.cache_read_input_tokens);
                    // 1) 普通输入至少 1，避免上报 0
                    assert!(
                        u.input_tokens >= 1,
                        "input_tokens<1: {source:?} upstream={upstream} local={total_local}"
                    );
                    // 2) 三项均非负
                    assert!(u.cache_creation_input_tokens >= 0 && u.cache_read_input_tokens >= 0);
                    // 3) 之和等于选定的总量（Local/Min 在本地口径为 0 时退回上游值）
                    let expected = match source {
                        InputTokenSource::Upstream => i64::from(upstream.max(1)),
                        _ if total_local == 0 => i64::from(upstream.max(1)),
                        InputTokenSource::Local => total_local,
                        InputTokenSource::Min => total_local.min(i64::from(upstream)).max(1),
                    };
                    assert_eq!(
                        sum, expected,
                        "sum mismatch: {source:?} upstream={upstream} local={total_local} got={u:?}"
                    );
                    // 4) 缓存读取不得超过总量
                    assert!(i64::from(u.cache_read_input_tokens) <= sum);
                }
            }
        }
    }

    #[test]
    fn disabled_mode_reports_all_as_plain_input() {
        use crate::model::config::PromptCacheMode;
        let payload = base_request(false);
        let split = simulate_with_mode(&payload, PromptCacheMode::Disabled);
        assert_eq!(split.read_local, 0);
        assert_eq!(split.creation_local, 0);
        assert_eq!(split.total_local, local_total(&payload));
        let usage = split.apply(1234);
        assert_eq!(usage.input_tokens, 1234);
        assert_eq!(usage.cache_creation_input_tokens, 0);
        assert_eq!(usage.cache_read_input_tokens, 0);
    }

    #[test]
    fn first_request_is_all_creation_then_repeat_is_all_read() {
        let cache = PromptCache::new();
        let payload = base_request(false);
        let total = local_total(&payload);
        let start = Instant::now();

        let first = cache.simulate_at(&payload, start);
        assert_eq!(first.total_local, total);
        assert_eq!(first.read_local, 0);
        assert_eq!(first.creation_local, total);

        let second = cache.simulate_at(&payload, start + Duration::from_secs(60));
        assert_eq!(second.read_local, total);
        assert_eq!(second.creation_local, 0);
    }

    #[test]
    fn longer_prefix_reads_old_part_and_creates_new_part() {
        let cache = PromptCache::new();
        let short = base_request(false);
        let long = base_request(true);
        let short_total = local_total(&short);
        let long_total = local_total(&long);
        assert!(long_total > short_total);
        let start = Instant::now();

        cache.simulate_at(&short, start);
        let split = cache.simulate_at(&long, start + Duration::from_secs(10));
        assert_eq!(split.read_local, short_total);
        assert_eq!(split.creation_local, long_total - short_total);
        assert_eq!(split.total_local, long_total);
    }

    #[test]
    fn expired_entry_is_created_again() {
        let cache = PromptCache::new();
        let payload = base_request(false);
        let total = local_total(&payload);
        let start = Instant::now();

        cache.simulate_at(&payload, start);
        let expired = cache.simulate_at(&payload, start + CACHE_TTL + Duration::from_secs(1));
        assert_eq!(expired.read_local, 0);
        assert_eq!(expired.creation_local, total);
    }

    #[test]
    fn hit_refreshes_ttl() {
        let cache = PromptCache::new();
        let payload = base_request(false);
        let start = Instant::now();

        cache.simulate_at(&payload, start);
        cache.simulate_at(&payload, start + Duration::from_secs(240));
        let split = cache.simulate_at(&payload, start + Duration::from_secs(480));
        assert_eq!(split.read_local, local_total(&payload));
    }

    #[test]
    fn only_last_four_breakpoints_count() {
        let cache = PromptCache::new();
        let block = |text: &str| json!({"type": "text", "text": long_text(text), "cache_control": {"type": "ephemeral"}});
        let payload = request(json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": [block("a"), block("b"), block("c"), block("d"), block("e")]}]
        }));
        cache.simulate_at(&payload, Instant::now());
        assert_eq!(cache.entries.lock().len(), MAX_BREAKPOINTS);
    }

    const SYSTEM_TEXTS: [&str; 2] = ["you are claude code", "project instructions"];

    /// 模拟 Claude Code：system 两块（第二块带断点）+ `message_count` 条单 block 消息，
    /// 只有最后一条消息的 block 带断点
    fn claude_code_request(message_count: usize) -> MessagesRequest {
        let messages: Vec<Value> = (0..message_count)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                let mut block = json!({"type": "text", "text": long_text(&format!("message {i}"))});
                if i + 1 == message_count {
                    block["cache_control"] = json!({"type": "ephemeral"});
                }
                json!({"role": role, "content": [block]})
            })
            .collect();
        request(json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "system": [
                {"type": "text", "text": long_text(SYSTEM_TEXTS[0])},
                {"type": "text", "text": long_text(SYSTEM_TEXTS[1]), "cache_control": {"type": "ephemeral"}}
            ],
            "messages": messages
        }))
    }

    fn system_prefix_tokens() -> i64 {
        SYSTEM_TEXTS.iter().map(|t| count(&long_text(t))).sum()
    }

    #[test]
    fn moved_breakpoint_reads_previous_turn_prefix() {
        let cache = PromptCache::new();
        let start = Instant::now();
        let round1 = claude_code_request(5);
        let round2 = claude_code_request(7);

        let first = cache.simulate_at(&round1, start);
        assert_eq!(first.read_local, 0);
        assert_eq!(first.creation_local, first.total_local);

        let second = cache.simulate_at(&round2, start + Duration::from_secs(30));
        assert!(first.total_local > system_prefix_tokens());
        assert_eq!(second.read_local, first.total_local);
        assert_eq!(
            second.creation_local,
            second.total_local - first.total_local
        );
        assert_eq!(second.total_local, local_total(&round2));
    }

    #[test]
    fn lookback_is_limited_to_twenty_boundaries() {
        let start = Instant::now();
        let round1 = claude_code_request(5);

        // 旧断点在本轮最后断点之前 20 个边界：仍在回看范围内，命中
        let cache = PromptCache::new();
        let first = cache.simulate_at(&round1, start);
        let within = cache.simulate_at(
            &claude_code_request(5 + 20),
            start + Duration::from_secs(30),
        );
        assert_eq!(within.read_local, first.total_local);

        // 旧断点在本轮最后断点之前 25 个边界：超出回看范围，回退到 system 断点
        let cache = PromptCache::new();
        cache.simulate_at(&round1, start);
        let round2 = claude_code_request(5 + 25);
        let beyond = cache.simulate_at(&round2, start + Duration::from_secs(30));
        assert_eq!(beyond.read_local, system_prefix_tokens());
        assert_eq!(
            beyond.creation_local,
            beyond.total_local - system_prefix_tokens()
        );
    }

    fn tool_use_block(id: &str) -> Value {
        json!({
            "type": "tool_use",
            "id": id,
            "name": "read_file",
            "input": {
                "path": format!("/workspace/src/{id}/module.rs"),
                "note": long_text("look at the surrounding code before editing"),
            }
        })
    }

    fn tool_result_block(id: &str) -> Value {
        json!({
            "type": "tool_result",
            "tool_use_id": id,
            "content": format!("fn {id}() {{ let value = compute(); }}\n").repeat(50)
        })
    }

    #[test]
    fn tool_blocks_are_counted_and_new_turn_is_creation() {
        let final_text = "please continue";
        let round1 = request(json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": [tool_result_block("toolu_a")]},
                {"role": "assistant", "content": [tool_use_block("toolu_b")]},
                {"role": "user", "content": [
                    {"type": "text", "text": final_text, "cache_control": {"type": "ephemeral"}}
                ]}
            ]
        }));
        assert!(
            tool_result_block("toolu_a")["content"]
                .as_str()
                .unwrap()
                .len()
                >= 2000
        );
        assert!(tool_use_block("toolu_b")["input"].to_string().len() >= 500);

        let new_tool_use = tool_use_block("toolu_c");
        let mut new_tool_result = tool_result_block("toolu_c");
        new_tool_result["cache_control"] = json!({"type": "ephemeral"});
        let round2 = request(json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": [tool_result_block("toolu_a")]},
                {"role": "assistant", "content": [tool_use_block("toolu_b")]},
                {"role": "user", "content": [{"type": "text", "text": final_text}]},
                {"role": "assistant", "content": [new_tool_use.clone()]},
                {"role": "user", "content": [new_tool_result.clone()]}
            ]
        }));

        let cache = PromptCache::new();
        let start = Instant::now();
        let first = cache.simulate_at(&round1, start);
        let text_only = count(final_text);
        assert!(
            first.total_local > text_only + 500,
            "total_local={} text_only={text_only}",
            first.total_local
        );
        assert_eq!(first.creation_local, first.total_local);

        let second = cache.simulate_at(&round2, start + Duration::from_secs(30));
        let expected_creation = block_tokens(&new_tool_use) + block_tokens(&new_tool_result);
        assert!(expected_creation > 0);
        assert_eq!(second.read_local, first.total_local);
        assert_eq!(second.creation_local, expected_creation);
        assert_eq!(second.total_local, first.total_local + expected_creation);
    }

    #[test]
    fn block_tokens_by_type() {
        let text = "some words to count here";
        let n = count(text);
        assert_eq!(block_tokens(&json!({"type": "text", "text": text})), n);
        assert_eq!(
            block_tokens(&json!({"type": "thinking", "thinking": text})),
            n
        );
        assert_eq!(
            block_tokens(&json!({"type": "redacted_thinking", "data": text})),
            n
        );
        assert_eq!(
            block_tokens(&json!({"type": "tool_result", "content": [
                {"type": "text", "text": text},
                {"type": "image", "source": {}}
            ]})),
            n + MEDIA_BLOCK_TOKENS
        );
        assert_eq!(
            block_tokens(&json!({"type": "document"})),
            MEDIA_BLOCK_TOKENS
        );
        assert_eq!(block_tokens(&json!({"type": "tool_use"})), 0);
        assert_eq!(block_tokens(&json!({"type": "unknown", "text": text})), 0);
        assert_eq!(block_tokens(&json!({"text": text})), 0);
    }

    #[test]
    fn apply_sums_to_actual_total_with_input_at_least_one() {
        let cases = [
            CacheSplit {
                total_local: 1000,
                read_local: 1000,
                creation_local: 0,
            },
            CacheSplit {
                total_local: 1000,
                read_local: 0,
                creation_local: 1000,
            },
            CacheSplit {
                total_local: 1000,
                read_local: 600,
                creation_local: 300,
            },
            CacheSplit {
                total_local: 1000,
                read_local: 0,
                creation_local: 0,
            },
            CacheSplit {
                total_local: 0,
                read_local: 0,
                creation_local: 0,
            },
        ];
        for split in cases {
            for actual in [-5, 0, 1, 2, 3, 7, 999, 1000, 1001, 50_000] {
                let usage = split.apply(actual);
                let sum = usage.input_tokens
                    + usage.cache_creation_input_tokens
                    + usage.cache_read_input_tokens;
                assert_eq!(sum, actual.max(1), "{split:?} actual={actual}");
                assert!(usage.input_tokens >= 1, "{split:?} actual={actual}");
                assert!(usage.cache_creation_input_tokens >= 0);
                assert!(usage.cache_read_input_tokens >= 0);
            }
        }
    }

    #[test]
    fn apply_scales_proportionally() {
        let split = CacheSplit {
            total_local: 1000,
            read_local: 600,
            creation_local: 300,
        };
        let larger = split.apply(2000);
        assert_eq!(larger.cache_read_input_tokens, 1200);
        assert_eq!(larger.cache_creation_input_tokens, 600);
        assert_eq!(larger.input_tokens, 200);
        let smaller = split.apply(100);
        assert_eq!(smaller.cache_read_input_tokens, 60);
        assert_eq!(smaller.cache_creation_input_tokens, 30);
        assert_eq!(smaller.input_tokens, 10);
        // 全部命中时给 input 让出 1
        let all_read = CacheSplit {
            total_local: 1000,
            read_local: 1000,
            creation_local: 0,
        }
        .apply(2);
        assert_eq!(
            (all_read.input_tokens, all_read.cache_read_input_tokens),
            (1, 1)
        );
    }

    #[test]
    fn usage_json_has_all_fields() {
        let usage = UsageTokens {
            input_tokens: 3,
            cache_creation_input_tokens: 5,
            cache_read_input_tokens: 7,
        };
        assert_eq!(
            usage.to_usage_json(11),
            json!({
                "input_tokens": 3,
                "cache_creation_input_tokens": 5,
                "cache_read_input_tokens": 7,
                "output_tokens": 11
            })
        );
    }
}
