# Connect GitHub

Pick a repository in the dashboard: glot opens a pull request with your app localized, ready to merge. No terminal, no CLI install, no API token to copy. After the merge, every push to your branch updates translations in a single `glot/update` pull request.

## How it works

1. **Connect GitHub** on the project's GitHub page (or in the setup checklist). GitHub asks you to install the glot app on your account or organization and to pick the repositories it may access.
2. **Pick a repository** and its base branch (your default branch, usually `main`).
3. glot runs a job you can watch live in the dashboard:

   | Step | What happens |
   |---|---|
   | Connecting to GitHub | A one-hour token limited to this one repository |
   | Cloning the repository | Shallow clone (`--depth 1`) in a temporary folder |
   | Extracting and translating | `glot init` if there is no `glot.toml`, then `glot localize --yes --no-install`, with your project's LLM settings and key |
   | Syncing catalogs | Same rules as `glot push`: reviewed or human translations in the dashboard are never overwritten |
   | Pushing the branch | One commit on `glot/localize` |
   | Opening the pull request | Coverage per language, strings converted, validation issues, changed files, next steps |

4. Review the pull request. If glot added an i18n library (next-intl, react-i18next…), the description tells you the exact command to run, for example `pnpm add next-intl`, because glot never runs package managers.
5. Merge. From then on each push to the base branch re-translates only what changed. There is **one open glot pull request at a time**: while one is open, glot updates it (and comments the new coverage); once it is merged or closed, the next change opens `glot/update`.

## What glot does not do

- It never runs your code: no `npm install`, no build, no scripts, no git hooks. It only reads and writes files with the `glot` binary.
- It never pushes to your base branch. Everything goes through a pull request.
- It does not push workflow files (the app has no `workflows` permission). To check every pull request with `glot check`, add the [GitHub Action](integrations.md) yourself.

## Permissions

| Permission | Why |
|---|---|
| Contents: read & write | Clone the repository and push the `glot/*` branch |
| Pull requests: read & write | Open and update the glot pull request, post status comments |
| Metadata: read | Required by GitHub for every app |

Webhooks: `push` (re-translate after a change on the base branch), `pull_request` (know when the glot PR is merged or closed), `installation` (forget repositories when the app is uninstalled).

## Security

- Webhooks are verified with their `X-Hub-Signature-256` HMAC; unsigned or wrongly signed deliveries are refused.
- Installation tokens live in memory for one job, are scoped to the linked repository, and reach git through an environment-provided header: never in a command line, a `.git/config` file or the logs.
- Which installations you can use is proven by GitHub itself (OAuth on install, then `GET /user/installations`), so nobody can attach someone else's installation to their project.
- Each job works in a dedicated temporary folder, deleted when it ends, with a size cap, timeouts, and memory/CPU limits on `glot`. Symbolic links are checked out as plain files, and a `glot.toml` whose catalog or source paths leave the repository is rejected.
- Runs are rate limited per installation, and pushes during a run are coalesced into one follow-up job.

## Self-hosting: create the GitHub App

Each glot server has its own GitHub App, created in two clicks with GitHub's [manifest flow](https://docs.github.com/en/apps/sharing-github-apps/registering-a-github-app-from-a-manifest):

1. Set `GLOT_ADMIN_EMAILS` (comma-separated) on the server and sign in with one of these emails.
2. Open `/admin/github-app`, choose a name (unique on GitHub) and optionally an organization, then **Create the GitHub App**. Confirm on GitHub.

GitHub sends the credentials back to glot: the app id, the private key, the webhook secret and the OAuth client secret are stored encrypted with `GLOT_SECRET_KEY`. The webhook URL is `{GLOT_PUBLIC_URL}/api/github/webhook`; install and OAuth callbacks go to `{GLOT_WEB_ORIGIN}/api/github/…`.

The server image must contain `git` and the `glot` binary (the official `crates/server/Dockerfile` ships both).

| Variable | Default | Purpose |
|---|---|---|
| `GLOT_ADMIN_EMAILS` | (none) | Who may create or replace the GitHub App |
| `GLOT_BIN` | `glot` | Path of the `glot` binary used by the jobs |
| `GLOT_GITHUB_MAX_REPO_MB` | `300` | Largest clone accepted (working tree + `.git`) |
| `GLOT_GITHUB_TIMEOUT_SECS` | `900` | Time limit of `glot localize` (also its CPU limit) |
| `GLOT_GITHUB_GIT_TIMEOUT_SECS` | `180` | Time limit of each git command |
| `GLOT_GITHUB_MEM_MB` | `1024` | Address-space limit of `glot` |
| `GLOT_GITHUB_JOBS_PER_HOUR` | `20` | Runs per installation per hour |
| `GLOT_GITHUB_API` / `GLOT_GITHUB_WEB` / `GLOT_GITHUB_GIT` | GitHub.com | API, web and git base URLs (GitHub Enterprise Server, tests) |

## Troubleshooting

- **"Add your LLM API key"**: glot translates with your project's key. Set it in the project settings, then **Run again**.
- **The repository is missing from the list**: give the app access to it on GitHub (the link under the list), then come back.
- **Installation requested**: in an organization, an owner must approve the app before you can pick its repositories.
- **`glot check` reports issues in the PR**: fix them in the dashboard (reviewed values are kept by the next run) or locally with `glot translate --force`.
