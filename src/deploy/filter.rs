use glob::Pattern;
use regex::Regex;

use crate::configure::{CommitFilter, FilterMode};
use crate::webhook::payload::{CommitInfo, PushEvent};

pub fn branch_matches(event: &PushEvent, expected: &str) -> bool {
    event.branch_name() == expected
}

/// Returns true if the deploy should proceed based on the commit filter.
/// No filter (no globs and no message patterns) → always proceed.
///
/// The file-glob check and the message-pattern check are evaluated
/// independently, each following the mode below, then combined with OR:
/// - Include mode → proceed if ANY changed file matches a glob, or ANY
///   commit message matches a pattern.
/// - Exclude mode → skip only if EVERY changed file matches a glob, or
///   EVERY commit message matches a pattern (i.e. skip when a check finds
///   nothing but excluded content).
///
/// A check with no configured globs/patterns contributes nothing (neither
/// forces a skip nor a proceed).
pub fn commit_filter_passes(commits: &[CommitInfo], filter: &CommitFilter) -> bool {
    let patterns: Vec<Pattern> = filter
        .globs()
        .iter()
        .filter_map(|g| Pattern::new(g).ok())
        .collect();
    let message_patterns: Vec<Regex> = filter
        .message_patterns()
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect();

    if patterns.is_empty() && message_patterns.is_empty() {
        return true;
    }

    let mut all_files = commits.iter().flat_map(CommitInfo::all_files).peekable();
    let mut all_messages = commits.iter().map(CommitInfo::message).peekable();

    match filter.mode() {
        FilterMode::Include => {
            let file_hit =
                !patterns.is_empty() && all_files.any(|f| patterns.iter().any(|p| p.matches(f)));
            let message_hit = !message_patterns.is_empty()
                && all_messages.any(|m| message_patterns.iter().any(|p| p.is_match(m)));
            file_hit || message_hit
        }
        FilterMode::Exclude => {
            let files_all_excluded = !patterns.is_empty()
                && all_files.peek().is_some()
                && all_files.all(|f| patterns.iter().any(|p| p.matches(f)));
            let messages_all_excluded = !message_patterns.is_empty()
                && all_messages.peek().is_some()
                && all_messages.all(|m| message_patterns.iter().any(|p| p.is_match(m)));
            !(files_all_excluded || messages_all_excluded)
        }
    }
}
