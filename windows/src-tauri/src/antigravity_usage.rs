//! Antigravity CLI (`agy`) token usage -> pet XP.
//!
//! agy's hook payloads carry no token counts, but every payload has
//! `transcriptPath`: `<brain>/<conversation>/.system_generated/logs/transcript_full.jsonl`,
//! one JSON object per step. Model steps (`PLANNER_RESPONSE`) carry
//! `input_tokens`, `cache_read_tokens` and `output_tokens`, written before the
//! hook that follows them fires. So this reads the new bytes since the last
//! call, like the Codex rollout reader (`transcript::new_codex_usage_tokens`).
//!
//! Counted like Codex: uncached input + output. agy reports no thinking or
//! cache-write split, so those are not counted.
//!
//! The first time a transcript is seen only the current request is counted
//! (everything from the last `USER_INPUT` step on), so resuming a long
//! conversation (`agy -c`) does not feed its whole history at once.
//!
//! simplify: a transcript is read whole once (first sighting), then only its
//! appended tail. Tokens burned before AgentPet first saw a conversation, in
//! earlier requests, are not backfilled.

use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

// Byte offset already consumed per transcript path.
static OFFSETS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Tokens one transcript step is worth (0 for steps without usage).
fn step_tokens(v: &Value) -> i64 {
    let get = |k: &str| v.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
    (get("input_tokens") - get("cache_read_tokens")).max(0) + get("output_tokens")
}

/// Sum of the tokens in complete JSONL lines.
pub fn sum_tokens(chunk: &[u8]) -> i64 {
    chunk
        .split(|b| *b == b'\n')
        .filter(|l| contains(l, b"\"input_tokens\"") || contains(l, b"\"output_tokens\""))
        .filter_map(|l| serde_json::from_slice::<Value>(l).ok())
        .map(|v| step_tokens(&v))
        .sum()
}

/// Byte offset of the line holding the last `USER_INPUT` step (0 if none):
/// where the current request starts.
pub fn current_request_start(buf: &[u8]) -> usize {
    let mut best = 0;
    let mut pos = 0;
    for line in buf.split_inclusive(|b| *b == b'\n') {
        if contains(line, b"USER_INPUT") {
            if let Ok(v) = serde_json::from_slice::<Value>(line) {
                if v.get("type").and_then(|t| t.as_str()) == Some("USER_INPUT") {
                    best = pos;
                }
            }
        }
        pos += line.len();
    }
    best
}

/// New tokens appended to the transcript since the previous call. `None` if the
/// file is unreadable, `Some(0)` if nothing new. Serialised across threads so
/// two events for one conversation cannot both consume the same bytes.
pub fn new_usage_tokens(path: &str) -> Option<i64> {
    let mut guard = OFFSETS.lock().ok()?;
    let offsets = guard.get_or_insert_with(HashMap::new);

    let mut f = std::fs::File::open(path).ok()?;
    let size = f.seek(SeekFrom::End(0)).ok()?;
    let known = offsets.get(path).copied();
    let mut start = known.unwrap_or(0);
    if start > size {
        start = 0; // file replaced: start over
    }
    if known.is_some() && size <= start {
        return Some(0);
    }
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;

    let skip = if known.is_none() { current_request_start(&buf) } else { 0 };
    let body = &buf[skip..];
    let Some(nl) = body.iter().rposition(|&b| b == b'\n') else {
        // No complete line yet: remember where the request starts, read later.
        offsets.insert(path.to_string(), start + skip as u64);
        return Some(0);
    };
    let consumable = &body[..=nl];
    offsets.insert(path.to_string(), start + (skip + consumable.len()) as u64);
    Some(sum_tokens(consumable))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shapes copied from a real transcript_full.jsonl (agy 2026-10-09).
    const USER: &str = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-10-09T03:59:10Z","content":"hi"}"#;
    const PLAN1: &str = r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-10-09T03:59:12Z","input_tokens":11641,"cache_read_tokens":0,"output_tokens":58,"tool_calls":[{"name":"run_command"}]}"#;
    const TOOL: &str = r#"{"step_index":2,"source":"MODEL","type":"GENERIC","status":"DONE","content":"output mentions input_tokens and USER_INPUT"}"#;
    const PLAN2: &str = r#"{"step_index":3,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","input_tokens":11786,"cache_read_tokens":0,"output_tokens":1,"content":"done"}"#;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("agy-usage-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_file(&d);
        d
    }

    #[test]
    fn sums_input_plus_output_over_planner_steps() {
        let t = format!("{USER}\n{PLAN1}\n{TOOL}\n{PLAN2}\n");
        assert_eq!(sum_tokens(t.as_bytes()), 11641 + 58 + 11786 + 1);
    }

    #[test]
    fn cached_input_is_not_counted() {
        let l = r#"{"type":"PLANNER_RESPONSE","input_tokens":1000,"cache_read_tokens":900,"output_tokens":5}"#;
        assert_eq!(sum_tokens(format!("{l}\n").as_bytes()), 100 + 5);
        let over = r#"{"input_tokens":10,"cache_read_tokens":50,"output_tokens":2}"#;
        assert_eq!(sum_tokens(format!("{over}\n").as_bytes()), 2);
    }

    #[test]
    fn text_mentioning_the_keys_is_ignored() {
        assert_eq!(sum_tokens(format!("{TOOL}\n").as_bytes()), 0);
        assert_eq!(sum_tokens(b"not json input_tokens\n{\"input_tokens\":3"), 0);
    }

    #[test]
    fn request_starts_at_last_user_input() {
        let old = format!("{USER}\n{PLAN1}\n");
        let t = format!("{old}{USER}\n{PLAN2}\n");
        assert_eq!(current_request_start(t.as_bytes()), old.len());
        assert_eq!(current_request_start(format!("{PLAN1}\n").as_bytes()), 0);
        // a tool output that merely mentions USER_INPUT is not a request start
        assert_eq!(current_request_start(format!("{USER}\n{TOOL}\n").as_bytes()), 0);
    }

    #[test]
    fn feeds_only_new_tokens_and_never_twice() {
        let p = tmp("incr");
        std::fs::write(&p, format!("{USER}\n")).unwrap();
        let ps = p.to_str().unwrap();
        assert_eq!(new_usage_tokens(ps), Some(0));
        let mut s = format!("{USER}\n{PLAN1}\n{TOOL}\n");
        std::fs::write(&p, &s).unwrap();
        assert_eq!(new_usage_tokens(ps), Some(11641 + 58));
        assert_eq!(new_usage_tokens(ps), Some(0)); // same bytes again: nothing
        s.push_str(PLAN2);
        s.push('\n');
        std::fs::write(&p, &s).unwrap();
        assert_eq!(new_usage_tokens(ps), Some(11786 + 1));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn resumed_conversation_does_not_feed_history() {
        let p = tmp("resume");
        // an old request (with usage) then the new request's user step
        std::fs::write(&p, format!("{USER}\n{PLAN1}\n{TOOL}\n{USER}\n")).unwrap();
        let ps = p.to_str().unwrap();
        assert_eq!(new_usage_tokens(ps), Some(0));
        std::fs::write(&p, format!("{USER}\n{PLAN1}\n{TOOL}\n{USER}\n{PLAN2}\n")).unwrap();
        assert_eq!(new_usage_tokens(ps), Some(11786 + 1));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn first_sighting_mid_request_counts_that_request_so_far() {
        let p = tmp("mid");
        std::fs::write(&p, format!("{USER}\n{PLAN1}\n{TOOL}\n")).unwrap();
        assert_eq!(new_usage_tokens(p.to_str().unwrap()), Some(11641 + 58));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn partial_last_line_waits_for_its_newline() {
        let p = tmp("partial");
        let ps = p.to_str().unwrap();
        std::fs::write(&p, format!("{USER}\n")).unwrap();
        assert_eq!(new_usage_tokens(ps), Some(0));
        std::fs::write(&p, format!("{USER}\n{}", &PLAN1[..60])).unwrap();
        assert_eq!(new_usage_tokens(ps), Some(0));
        std::fs::write(&p, format!("{USER}\n{PLAN1}\n")).unwrap();
        assert_eq!(new_usage_tokens(ps), Some(11641 + 58));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn replaced_file_starts_over_and_missing_file_is_none() {
        let p = tmp("replace");
        let ps = p.to_str().unwrap();
        std::fs::write(&p, format!("{USER}\n{PLAN1}\n{PLAN1}\n")).unwrap();
        assert_eq!(new_usage_tokens(ps), Some(2 * (11641 + 58)));
        std::fs::write(&p, format!("{USER}\n{PLAN2}\n")).unwrap(); // shorter: replaced
        assert_eq!(new_usage_tokens(ps), Some(11786 + 1));
        let _ = std::fs::remove_file(&p);
        assert_eq!(new_usage_tokens("Z:/no/such/transcript.jsonl"), None);
    }
}
