use glob::Pattern;

use crate::configure::{CommitFilter, FilterMode};
use crate::webhook::payload::{CommitInfo, PushEvent};

pub fn branch_matches(event: &PushEvent, expected: &str) -> bool {
    event.branch_name() == expected
}

/// Returns true if the deploy should proceed based on the commit filter.
/// No filter → always proceed.
/// Include mode → proceed if ANY commit touches ANY matching glob.
/// Exclude mode → proceed if NO commit touches ANY matching glob.
pub fn commit_filter_passes(commits: &[CommitInfo], filter: &CommitFilter) -> bool {
    let patterns: Vec<Pattern> = filter
        .globs()
        .iter()
        .filter_map(|g| Pattern::new(g).ok())
        .collect();

    if patterns.is_empty() {
        return true;
    }

    let any_matches = commits.iter().any(|commit| {
        commit
            .all_files()
            .any(|file| patterns.iter().any(|p| p.matches(file)))
    });

    match filter.mode() {
        FilterMode::Include => any_matches,
        FilterMode::Exclude => !any_matches,
    }
}
