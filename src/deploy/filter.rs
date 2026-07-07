use glob::Pattern;

use crate::configure::{CommitFilter, FilterMode};
use crate::webhook::payload::{CommitInfo, PushEvent};

pub fn branch_matches(event: &PushEvent, expected: &str) -> bool {
    event.branch_name() == expected
}

/// Returns true if the deploy should proceed based on the commit filter.
/// No filter → always proceed.
/// Include mode → proceed if ANY changed file matches a glob.
/// Exclude mode → proceed only if NOT every changed file matches a glob
/// (i.e. skip only when the push touches nothing but excluded files).
pub fn commit_filter_passes(commits: &[CommitInfo], filter: &CommitFilter) -> bool {
    let patterns: Vec<Pattern> = filter
        .globs()
        .iter()
        .filter_map(|g| Pattern::new(g).ok())
        .collect();

    if patterns.is_empty() {
        return true;
    }

    let mut all_files = commits.iter().flat_map(CommitInfo::all_files).peekable();

    match filter.mode() {
        FilterMode::Include => all_files.any(|file| patterns.iter().any(|p| p.matches(file))),
        FilterMode::Exclude => {
            all_files.peek().is_none()
                || !all_files.all(|file| patterns.iter().any(|p| p.matches(file)))
        }
    }
}
