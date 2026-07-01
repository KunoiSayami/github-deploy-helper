# Github Deploy Helper

A lightweight Rust server that listens for GitHub push webhooks and runs deployment pipelines on your server. Supports multiple projects on a single port, per-project configuration, Telegram notifications, and optional log cleanup.

## Features

- **HMAC-SHA256 verification** — rejects any request not signed by GitHub
- **Multi-project** — each project gets its own URL path (`/webhook/<name>`)
- **Deployment pipeline** — `stop → git pull → init (first deploy) → update → start`
- **Branch and commit filters** — include/exclude deploys by branch or changed file globs
- **Per-project `deploy.toml`** — override commands and settings per repo without touching the main config
- **Deployment locking** — only one deploy per project at a time (disable with `--no-lock`)
- **Telegram notifications** — notified on deploy start, success, and failure
- **Log rolling and cleanup** — daily rolling deploy log with configurable retention

## Installation

```sh
git clone <this-repo>
cd github-deploy-helper
cp data/config.toml.default data/config.toml
# edit data/config.toml
cargo build --release
./target/release/github-deploy-helper
```

## Configuration

Copy `data/config.toml.default` to `data/config.toml` and edit it. A different path can be specified with `-c`.

### Global options

```toml
bind            = "0.0.0.0:9000"   # listen address
log_dir         = "logs"           # directory for deploy.log files
default_timeout = 30               # command timeout in seconds (per-project override available)
log_keep_days   = 30               # delete log files older than this many days; 0 = keep forever
```

### Telegram (optional)

```toml
[telegram]
bot_token  = "123456:ABC..."
# api_server = "https://your-custom-api-server.example.com"
send_to    = [-100123456789]       # chat ID — integer, string, or array
```

Omit the `[telegram]` section entirely to disable notifications.

### Projects

Each project is a `[[projects]]` entry:

```toml
[[projects]]
name        = "my-api"             # used as the URL slug (/webhook/my-api) and log label
working_dir = "/srv/my-api"        # where commands are executed
secret      = "webhook-secret"     # GitHub webhook secret — NEVER put this in deploy.toml
branch      = "main"               # only deploy pushes to this branch
timeout     = 60                   # optional; overrides default_timeout
bypass      = false                # if true, return 200 but skip the deploy
deploy_toml = false                # if true, also load working_dir/deploy.toml for overrides

[projects.commit_filter]           # optional
mode  = "exclude"                  # "include" — only deploy if a commit touches these paths
                                   # "exclude" — skip deploy if all commits only touch these paths
globs = ["docs/**", "*.md"]

[projects.commands]
stop    = "systemctl stop my-api"
init    = "cargo build --release"  # runs only on the first deploy (or after --force-init)
update  = "cargo build --release"  # runs on every deploy
start   = "systemctl start my-api"
# restart = "systemctl restart my-api"   # if set, replaces stop + start
# pull    = "git pull --ff-only"         # default; override if needed
# no_pull = true                         # if true, skip the pull step entirely (webhook-only mode)
```

#### Per-project `deploy.toml`

When `deploy_toml = true`, the server reads `<working_dir>/deploy.toml` and merges it on top of the inline config. This lets the repo itself define build commands while keeping secrets in the main config.

```toml
# working_dir/deploy.toml
branch  = "main"
timeout = 120

[commit_filter]
mode  = "exclude"
globs = ["docs/**", "*.md"]

[commands]
stop   = "systemctl stop my-api"
init   = "./scripts/init.sh"
update = "./scripts/build.sh"
start  = "systemctl start my-api"
```

`secret` is intentionally **not** allowed in `deploy.toml` — keep credentials out of the repository.

## Deployment pipeline

For each qualifying push:

```
stop              ← optional; skipped if restart is set
git pull          ← runs by default (git pull --ff-only); skipped entirely if no_pull = true
init              ← only on first deploy, or after --force-init
update            ← runs on every deploy
start             ← optional; skipped if restart is set
restart           ← replaces stop + start when configured
```

If any step fails (non-zero exit or timeout), the deploy pauses. The pipeline will retry from the top on the next webhook delivery.

### Webhook-only mode (no pulling)

For projects where you don't want the server to touch git at all — e.g. deploys are handled some other way, or you just want a webhook to trigger a script — set `no_pull = true`. The `pull` step is skipped entirely and the pipeline becomes `stop → init (first delivery) → update → start`, driven purely by the webhook event (branch and commit filters still apply). Leave any of `stop`/`init`/`update`/`start` unset to skip that step too.

```toml
[projects.commands]
no_pull = true
update  = "./scripts/on-push.sh"
```

## CLI flags

```
github-deploy-helper [OPTIONS]

Options:
  -c, --config <FILE>   Config file [default: data/config.toml]
  --no-lock             Allow concurrent deploys per project (useful for testing)
  --force-init          Re-run the init command on the next deploy for all projects
  -h, --help            Print help
  -V, --version         Print version
```

## GitHub webhook setup

1. Go to your repository → **Settings → Webhooks → Add webhook**
2. Set **Payload URL** to `http://your-server:9000/webhook/<project-name>`
3. Set **Content type** to `application/json`
4. Set **Secret** to match the `secret` in your config
5. Select **Just the push event**

## Logging

- **Program logs** (startup, requests, errors) → stdout/stderr
- **Deploy execution logs** (commands, output, outcomes) → `<log_dir>/deploy.log.<date>` (daily rolling)

The `log_keep_days` setting automatically removes log files older than the specified number of days. Set to `0` to disable cleanup.

## License

AGPL-3.0
