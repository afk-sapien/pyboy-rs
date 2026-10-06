# Project rules

## Project identity

All agents and automation acting on the owner's behalf must use only this
identity for this project:

- GitHub account and Git author/committer name: `afk-sapien`
- Git author/committer email: `327645577+afk-sapien@users.noreply.github.com`

Never use a work account, a work email, another saved login, or an agent's
suggested co-author identity for this work. This applies to commits, tags,
co-author and sign-off trailers, pull requests, merges, releases, issues,
comments, package publication, and other authenticated project operations.
Preserve legitimate third-party attribution in upstream code and history.

### Every checkout, clone, and worktree

These rules also apply to temporary clones, isolated checkouts, worktrees,
subprojects, and delegated work, regardless of their filesystem location.
A fresh clone does not inherit the source checkout's local Git configuration.
Before creating commits or tags in each checkout, set repository-local values:

```sh
git config --local user.name afk-sapien
git config --local user.email 327645577+afk-sapien@users.noreply.github.com
git config --local user.useConfigOnly true
```

Verify the effective identity with both `git var GIT_AUTHOR_IDENT` and
`git var GIT_COMMITTER_IDENT`. Both must have the exact name and email above.
Check for environment variables, command options, and worktree configuration
that override local settings. Recheck after changing checkouts or execution
environments. Do not change global Git identity or the global active login.
Other projects may intentionally use a different account.

### Authentication and publication

Git authorship and service authentication are separate checks. Before any
authenticated project operation, verify the actual account used by that CLI,
connector, browser, credential helper, or API client is `afk-sapien`. For GitHub
CLI, `gh api user --jq .login` must return `afk-sapien`. Ensure Git pushes use
that same verified account. A repository URL, a successful push, or a
`github.account` setting alone does not prove the effective identity.
Use credentials scoped to this repository or operation. Never print tokens.

If the required account is unavailable or either identity check fails, stop
the authenticated or history-writing operation and report the mismatch.
Never fall back to another saved account or bypass a project identity guard.

Before pushing, inspect every new commit's author, committer, and attribution
trailers. Before merging or publishing, inspect the final merge or squash
message and tag attribution too. Correct unintended identities before they
become public. Pass these rules explicitly to delegated agents and temporary
checkouts that might not load this file automatically.

## Project conventions

- Never use em dashes or semicolons in authored output.

Required Rust syntax may use semicolons. Preserve upstream attribution.
Never include commercial ROMs, checkpoints, credentials, or private game data.
