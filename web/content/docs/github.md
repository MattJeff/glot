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

## Keyless Claude access with Workload Identity Federation

The [GitHub Action](integrations.md#github-action) and the CLI can call Claude without an `ANTHROPIC_API_KEY` secret. Each run gets a signed OIDC token from GitHub, glot exchanges it for an Anthropic access token that lasts a few minutes, and nothing long-lived is stored in the repository. The dashboard (app.glotlang.com) is different: it has no OIDC identity of its own, so its jobs keep using the key stored in the project settings.

### 1. Create three resources in the Claude Console

Go to **Workload identity → Connect workload → GitHub Actions** (admin role required), or run `glot wif` (below).

| Resource | Value |
|---|---|
| Service account (`svac_…`) | e.g. `glot-ci`, role `developer`, member of the workspace that pays |
| Issuer (`fdis_…`) | Issuer URL `https://token.actions.githubusercontent.com`, JWKS **discovery**, **JTI check on** (the default), max JWT lifetime **1 h** (the default) |
| Rule (`fdrl_…`) | `subject_prefix` `repo:OWNER/REPO:*` plus the claim `repository_owner = OWNER`, audience `https://api.anthropic.com`, scope **`workspace:inference`** (Messages + Models, all glot needs), your workspace |

To limit the rule to one branch, add the claim `ref = refs/heads/main`, or a CEL condition such as `claims.ref in ["refs/heads/main", "refs/heads/release"]`. On `pull_request` runs, `sub` is `repo:OWNER/REPO:pull_request` and `ref` is `refs/pull/N/merge`, so a rule pinned to `main` does not match PR runs. Workflows triggered by fork PRs never get `id-token: write`. Never use `repo:OWNER/*`: that pattern matches every repository in the organization.

### 2. Workflow

```yaml
permissions:
  contents: write
  pull-requests: write
  id-token: write
jobs:
  i18n:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          repository: ${{ github.event.pull_request.head.repo.full_name }}
          ref: ${{ github.head_ref }}
      - uses: MattJeff/glot/action@v0
        with:
          command: ${{ github.event.pull_request.head.repo.full_name == github.repository && 'both' || 'check' }}
          federation-rule-id: fdrl_…
          organization-id: 00000000-0000-0000-0000-000000000000
          service-account-id: svac_…
          # workspace-id: wrkspc_…   # only if the rule spans several workspaces
```

These IDs are not secrets. The `.github/workflows/glot.yml` that `glot init` writes includes this as a commented-out example.

### Environment variables (CLI, any CI)

glot reads the same variables as the official Anthropic SDKs:

| Variable | |
|---|---|
| `ANTHROPIC_FEDERATION_RULE_ID` | `fdrl_…` |
| `ANTHROPIC_ORGANIZATION_ID` | Organization UUID |
| `ANTHROPIC_SERVICE_ACCOUNT_ID` | `svac_…` |
| `ANTHROPIC_IDENTITY_TOKEN_FILE` or `ANTHROPIC_IDENTITY_TOKEN` | The IdP's JWT. The file is read again on every exchange, so rotated projected tokens are picked up |
| `ANTHROPIC_WORKSPACE_ID` | Optional, required when the rule spans several workspaces |
| `ANTHROPIC_FEDERATION_AUDIENCE` | GitHub fallback only: audience requested (default `https://api.anthropic.com`) |

On GitHub Actions without a token file, glot requests a **fresh** OIDC token from `ACTIONS_ID_TOKEN_REQUEST_URL` for each exchange. It does this because GitHub tokens carry a single-use `jti` and expire after about 5 minutes. The access token is cached in memory and exchanged again 60 s before it expires.

Precedence matches the SDKs: an API key (`ANTHROPIC_API_KEY` or `llm.api_key_env`) wins, then `ANTHROPIC_AUTH_TOKEN` (Bearer), then federation. Unset leftover keys when you migrate. If the exchange returns a 401, it means *Authentication failed*: open **Console → Workload identity → Authentication history** to see the reason (`iss`/`aud`/`sub` mismatch, an expired JWT, a replayed `jti`).

### JWKS source of an issuer

| Mode | Fields | Use it for |
|---|---|---|
| `discovery` (default) | optional `discovery_base` when the discovery document is not under the issuer URL | Public OIDC providers: GitHub Actions, EKS, GKE, Entra ID |
| `explicit_url` | `url` of the JWKS | Discovery is internal-only but the JWKS endpoint is public |
| `inline` | `keys`: the **array** from a JWKS document (not the `{"keys": …}` wrapper) | Self-hosted or air-gapped Kubernetes. **No automatic refresh: update the issuer on every key rotation** |

`discovery` and `explicit_url` accept an optional `ca_cert_pem` (at most 8 KiB) for a JWKS served behind a private CA. Every URL Anthropic fetches must use `https` on port 443 with a public DNS hostname, and IP literals are refused. In `explicit_url` and `inline` modes, the issuer URL is only compared with `iss`, so it may be internal. `check_jti` (on by default) makes each JWT that carries a `jti` single-use, and turning it off removes replay protection for every rule on the issuer. `max_jwt_lifetime_seconds` (default 3600) caps `exp − iat`.

### `glot wif`

`glot wif` creates the issuer and the rule through the Admin API. These endpoints accept only an `org:admin` OAuth token, never an API key, not even an Admin key. Provide one with `ant auth login --profile admin --scope org:admin` and `export ANTHROPIC_AUTH_TOKEN=$(ant auth print-credentials --profile admin --access-token)`, or with the WIF env of a Console-created `org:admin` rule. Every command validates its input locally first: names match `^[a-z0-9-]+$`, URLs are https on port 443 with no IP, and token lifetimes are 60 to 86400 s. Add `--dry-run` to print the request body without calling the API.

```sh
glot wif issuer create --preset github                      # discovery, JTI on
glot wif issuer create --name k8s --issuer-url https://kubernetes.default.svc \
  --jwks-file jwks.json                                     # inline (or --jwks-from <url>)
glot wif issuer create --name corp --issuer-url https://idp.internal \
  --jwks-url https://keys.example.com/jwks.json --ca-cert ca.pem
glot wif rule create --preset github --repo OWNER/REPO --name glot-ci \
  --issuer fdis_… --service-account svac_… --workspace wrkspc_… --token-lifetime 600
glot wif check jwt.txt --preset github --repo OWNER/REPO    # decode locally, never sent
```

Other options: `--jwks discovery|explicit-url|inline`, `--discovery-base`, `--no-check-jti`, `--max-lifetime`, `--subject-prefix`, `--audience`, `--claim k=v` (repeatable), `--condition <CEL>`, `--scope workspace:inference|workspace:developer`, `--all-workspaces`, `--ref refs/heads/main`. `glot wif check` compares `iss`, `aud`, `sub`, the claims, `exp`/`iat`/lifetime, `kid` and `alg` against the issuer and rule. The signature itself is verified by Anthropic.
