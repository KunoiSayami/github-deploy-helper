# Github Deploy Helper

A lightweight Rust server that listens for GitHub push webhooks and runs deployment pipelines on your server. Supports multiple projects on a single port, per-project configuration, Telegram notifications, and optional log cleanup.

## Features

- **HMAC-SHA256 verification** — rejects any request not signed by GitHub
- **Multi-project** — each project gets its own URL path (`/webhook/<name>`)
- **Deployment pipeline** — `stop → git pull → init (first deploy) → update → start`
- **Branch and commit filters** — include/exclude deploys by branch, changed file globs, or commit message regex
- **Per-project `deploy.toml`** — override commands and settings per repo without touching the main config
- **Deployment locking** — only one deploy per project at a time (disable with `--no-lock`)
- **Telegram notifications** — notified on deploy start (with a commit summary), success, and failure
- **Log rolling and cleanup** — daily rolling deploy log with configurable retention
- **GitHub App authentication** — optional per-project alternative to SSH deploy keys for the `pull` step

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

Three events are sent per deploy:

- **Started** — includes a commit summary (linked commit IDs and first line of each message, or a linked "N new commits" comparison for multi-commit pushes)
- **Succeeded**
- **Failed** — includes the failing step and its output

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
mode  = "exclude"                  # "include" — only deploy if a commit touches these paths, or a commit message matches message_patterns
                                   # "exclude" — skip deploy only if every changed file matches globs, or every commit message matches message_patterns
globs = ["docs/**", "*.md"]
# message_patterns = ["^chore:", "\\[skip deploy\\]"]   # optional; regex, matched against each commit message

[projects.commands]
stop    = "systemctl stop my-api"
init    = "cargo build --release"  # runs only on the first deploy (or after --force-init)
update  = "cargo build --release"  # runs on every deploy
start   = "systemctl start my-api"
# restart = "systemctl restart my-api"   # if set, replaces stop + start
# pull    = "git fetch origin && git reset --hard origin/$(git rev-parse --abbrev-ref HEAD)"  # default; override if needed
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
# message_patterns = ["^chore:", "\\[skip deploy\\]"]

[commands]
stop   = "systemctl stop my-api"
init   = "./scripts/init.sh"
update = "./scripts/build.sh"
start  = "systemctl start my-api"
```

`secret` is intentionally **not** allowed in `deploy.toml` — keep credentials out of the repository.

### GitHub App authentication (optional, per-project)

By default, the `pull` command authenticates however the host's git/SSH setup is configured — typically a per-repo SSH deploy key. That's fine for a handful of projects, but managing one keypair + deploy key + SSH config entry per repo gets tedious as project count grows.

As an alternative, a project can authenticate via a GitHub App installation instead. This preserves per-repo isolation (the App is installed per-repo/org, same as a deploy key) while replacing key management with one App registration and one private key, shared across all projects that opt in — and tokens auto-expire in about an hour instead of being long-lived.

**This is opt-in and additive.** Projects that don't set `[projects.auth]` behave exactly as before — nothing changes for existing SSH-based configs.

1. **Register a GitHub App** (Settings → Developer settings → GitHub Apps → New GitHub App, either under your personal account or an org):
   - Permissions: **Repository permissions → Contents: Read-only** is sufficient for pulling.
   - Permissions: **Repository permissions → Webhooks: Read and write** — only needed if you want the server to auto-create the repo's webhook (see `public_base_url` below); skip it if you'll configure webhooks manually.
   - Generate a **private key** (downloads a `.pem` file) and note the **App ID**.
   - **Install** the App on each org/account whose repos you want to deploy — you can install the same App into multiple orgs; each installation is scoped to the repos you select there.

2. **Add the App config** (once, shared across all projects):

   ```toml
   [github_app]
   app_id           = 123456
   private_key_path = "/etc/github-deploy-helper/app-private-key.pem"
   # Optional: if set, the server auto-creates a `push` webhook on each github_app-authenticated
   # project's repo at startup, pointed at "<public_base_url><project webhook path>". Requires the
   # App's "Webhooks" permission (see above). Omit this to configure webhooks manually instead.
   public_base_url = "https://deploy.example.com"
   ```

3. **Opt a project in** by adding `[projects.auth]`:

   ```toml
   [projects.auth]
   mode  = "github_app"
   owner = "my-org"           # the org/user that owns the repo
   repo  = "my-api"           # the repo name
   ```

   `owner`/`repo` are used to resolve which App installation to use and to request a scoped installation token — they don't need to match `name` or `working_dir`.

4. **Reference the token in your `pull` command.** The server resolves and refreshes the installation token automatically and exposes it to the pull command as the `GH_TOKEN` environment variable (never interpolated into the command string, so it won't show up in `ps`/argv):

   ```toml
   [projects.commands]
   pull = "git -c http.extraHeader=\"AUTHORIZATION: basic $(printf 'x-access-token:%s' \"$GH_TOKEN\" | base64 -w0)\" fetch origin && git reset --hard origin/$(git rev-parse --abbrev-ref HEAD)"
   ```

   This requires the repo's `origin` remote to use an `https://github.com/...` URL rather than `git@github.com:...`.

`auth` can also be set/overridden in a project's `deploy.toml`, following the same override rules as `commands` and `commit_filter`.

## Deployment pipeline

For each qualifying push:

```
stop              ← optional; skipped if restart is set
git pull          ← runs by default (fetch + hard reset to origin, survives force-pushes); skipped entirely if no_pull = true
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

- **Program logs** (startup, requests, step summaries, errors) → stdout/stderr and `<log_dir>/deploy.log.<date>` (daily rolling)
- **Deploy command output** (raw stdout/stderr of each stop/pull/init/update/start/restart run) → `<log_dir>/<project-name>.log` (append-only, one file per project)

The `log_keep_days` setting automatically removes log files older than the specified number of days (based on last-modified time) from `log_dir`, including per-project logs. Set to `0` to disable cleanup.

## License

AGPL-3.0
