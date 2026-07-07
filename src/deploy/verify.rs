use serde::Deserialize;

use crate::auth::github_app::GithubAppAuth;
use crate::configure::CommitVerify;
use crate::webhook::payload::{CommitInfo, PushEvent};

/// Returns true if every commit's author/committer identity (email or username)
/// is present in `verify`'s allowlist. An empty allowlist allows everything.
pub fn authors_allowed(commits: &[CommitInfo], verify: &CommitVerify) -> bool {
    let allowed = verify.allowed_authors();
    if allowed.is_empty() {
        return true;
    }
    commits.iter().all(|c| {
        c.identities()
            .any(|id| allowed.iter().any(|a| a.eq_ignore_ascii_case(id)))
    })
}

#[derive(Deserialize)]
struct CommitVerificationWrapper {
    commit: CommitDetail,
}

#[derive(Deserialize)]
struct CommitDetail {
    verification: Verification,
}

#[derive(Deserialize)]
struct Verification {
    verified: bool,
}

/// Checks, via the GitHub REST API, that every commit in this push is GPG/SSH-signed.
/// Requires the push event to carry a `repository.full_name` of the form `owner/repo`.
/// Uses the GitHub App installation token when available (higher rate limit, works on
/// private repos); otherwise falls back to an unauthenticated request.
pub async fn all_commits_signed(
    event: &PushEvent,
    github_app: Option<&GithubAppAuth>,
) -> anyhow::Result<bool> {
    let Some((owner, repo)) = event.repository().owner_repo() else {
        anyhow::bail!("cannot verify commit signatures: repository.full_name is not owner/repo");
    };

    let client = reqwest::Client::builder()
        .user_agent("github-deploy-helper")
        .build()?;

    let token = match github_app {
        Some(app) => Some(app.get_token(owner, repo).await?),
        None => None,
    };

    for commit in event.commits() {
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/commits/{}",
            commit.id()
        );
        let mut req = client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(t) = &token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await?.error_for_status()?;
        let parsed: CommitVerificationWrapper = resp.json().await?;
        if !parsed.commit.verification.verified {
            return Ok(false);
        }
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verify_with(allowed: &[&str]) -> CommitVerify {
        CommitVerify::for_test(allowed.iter().map(|s| s.to_string()).collect(), false)
    }

    fn commit_with_email(email: &str) -> CommitInfo {
        CommitInfo::for_test(Some(email.to_string()), None)
    }

    #[test]
    fn empty_allowlist_allows_everything() {
        let verify = verify_with(&[]);
        let commits = vec![commit_with_email("anyone@example.com")];
        assert!(authors_allowed(&commits, &verify));
    }

    #[test]
    fn matching_author_email_passes() {
        let verify = verify_with(&["alice@example.com"]);
        let commits = vec![commit_with_email("Alice@Example.com")];
        assert!(authors_allowed(&commits, &verify));
    }

    #[test]
    fn non_matching_author_email_fails() {
        let verify = verify_with(&["alice@example.com"]);
        let commits = vec![commit_with_email("mallory@example.com")];
        assert!(!authors_allowed(&commits, &verify));
    }

    #[test]
    fn one_unauthorized_commit_fails_whole_push() {
        let verify = verify_with(&["alice@example.com"]);
        let commits = vec![
            commit_with_email("alice@example.com"),
            commit_with_email("mallory@example.com"),
        ];
        assert!(!authors_allowed(&commits, &verify));
    }
}
