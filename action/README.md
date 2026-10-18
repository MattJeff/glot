# glot GitHub Action

Validates i18n catalogs with `glot check` (inline annotations), optionally translates the delta with `glot translate`, commits the result to the PR branch and keeps one summary comment up to date.

```yaml
on: pull_request
permissions:
  contents: write        # push the translation commit (command: translate | both)
  pull-requests: write   # post / update the summary comment (comment: true)
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
          # Fork PRs get no secrets: only check them, translate on same-repo branches.
          command: ${{ github.event.pull_request.head.repo.full_name == github.repository && 'both' || 'check' }}
          anthropic-api-key: ${{ secrets.ANTHROPIC_API_KEY }}
```

## Inputs

| Input | Default | |
|---|---|---|
| `command` | `check` | `check`, `translate` or `both` |
| `working-directory` | `.` | Directory containing `glot.toml` |
| `version` | pinned (`action/VERSION`) | glot release tag, or `latest`. Skipped when `glot` is already on PATH |
| `anthropic-api-key` | | Exported as `ANTHROPIC_API_KEY` |
| `openai-api-key` | | Exported as `OPENAI_API_KEY` (`openai`, `openai-compatible`) |
| `llm-api-key` | | Generic key, exported under `llm.api_key_env` from `glot.toml` (else the provider's default variable) |
| `anthropic-workspace-id` | | Exported as `ANTHROPIC_WORKSPACE_ID` (required by org-level user keys `sk-ant-usr-…`) |
| `comment` | `true` | Post or update the summary comment |
| `fail-on-error` | `true` | Fail the job when `glot check` reports errors |
| `github-token` | `github.token` | Used to comment on the PR |
| `token` | | Used to push the translation commit (PAT or GitHub App token); empty = checkout credentials |

Output: `check-exit-code`.

Empty key inputs leave the job's environment alone, so `env: ANTHROPIC_API_KEY: …` at job level works too.

## Supply chain

- The binary version is pinned to the action release: `@v0` installs the version in [`VERSION`](VERSION), not whatever is latest. Pass `version: latest` (or a tag) to override.
- The downloaded archive is checked against the release's `SHA256SUMS` asset. A missing file or a mismatch fails the job; nothing is installed unverified.
- Runners without a prebuilt binary build the same tag from source with `cargo install --locked`.

## What gets committed

Only what glot writes: the catalog files matching `files` in `glot.toml` (`{locale}` → any locale), `glot.lock` and `.glot/layout.json`. Never `git add -A`: other files changed earlier in the job stay out of the commit.

## Pushes with `GITHUB_TOKEN` do not trigger CI

GitHub does not start new workflow runs for commits pushed with the default `GITHUB_TOKEN`. After the action pushes translations, the PR's head commit has no checks: required status checks stay pending.

To have CI run on the translation commit, push with a PAT or a GitHub App token:

```yaml
      - uses: actions/create-github-app-token@v1
        id: app
        with:
          app-id: ${{ vars.GLOT_APP_ID }}
          private-key: ${{ secrets.GLOT_APP_KEY }}
      - uses: MattJeff/glot/action@v0
        with:
          command: both
          anthropic-api-key: ${{ secrets.ANTHROPIC_API_KEY }}
          token: ${{ steps.app.outputs.token }}   # needs contents: write on the repo
```

A fine-grained PAT with *Contents: read and write* on the repository works the same way (`token: ${{ secrets.GLOT_PUSH_TOKEN }}`). Watch for loops: the translation commit then triggers this workflow again, which finds nothing to translate and pushes nothing.

## Permissions

| Permission | Needed for |
|---|---|
| `contents: read` | `command: check` |
| `contents: write` | `command: translate` / `both` (push), unless `token` is set |
| `pull-requests: write` | `comment: true` |

Fork PRs get a read-only token: the comment falls back to the job summary instead of failing.
