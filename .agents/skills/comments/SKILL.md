---
name: comments
description: Fetch review feedback on the current branch's pull request, apply valid fixes as focused commits, push, reply with commit links and resolve fixed threads.
argument-hint: ''
triggers:
  - user
---

Read `AGENTS.md`, then `.agents/skills/git/SKILL.md` first. Follow the Git rules there for every commit and push.

Target: the open pull request whose head branch is the current local branch. This workflow takes no PR number, URL or branch override, because it commits and pushes to the checked-out branch.

## Contract

| Question | Answer |
| --- | --- |
| Scope | Open review threads plus standalone review or PR comments with actionable feedback |
| Mutations | Edit files, create focused commits, push, reply with commit links, resolve only fixed review threads |
| Boundary | Read every comment in full before triage; never apply comments blindly; never force-push; never close false-positive, skipped, question or standalone comments |

## 1. State

```bash
git status --short --branch
git log --oneline --decorate -10
gh pr status
gh pr view --json number,title,body,state,baseRefName,headRefName,headRefOid,url,reviewDecision,comments,reviews,files
```

Stop and ask when there is no open PR for the branch, when `headRefName` does not match the current branch, or when the worktree has unrelated local changes.

## 2. Collect feedback in full

Use GraphQL so inline threads, unanchored comments, suggestions, thread ids and numeric `databaseId` values are all available:

```bash
gh api graphql -F owner='<owner>' -F repo='<repo>' -F number=<number> -f query='query($owner:String!, $repo:String!, $number:Int!) { repository(owner:$owner, name:$repo) { pullRequest(number:$number) { reviewThreads(first:100) { pageInfo { hasNextPage endCursor } nodes { id isResolved isOutdated path line originalLine diffSide comments(first:100) { nodes { id databaseId author { login } body createdAt url path line diffHunk } } } } reviews(first:100) { nodes { id databaseId author { login } state body submittedAt url } } comments(first:100) { nodes { id databaseId author { login } body createdAt url } } } } }'
```

Page with `after: <endCursor>` while `hasNextPage` is true. Targets are unresolved review threads and standalone review or PR comments with feedback. Read each target's full body, details sections, suggestions, diff hunk and follow-up replies before triage.

## 3. Triage one target at a time

Read the referenced code at the current PR head, check sibling patterns, then classify:

- `apply`: valid, actionable and in scope;
- `false-positive`: contradicted by the code, repository rules or product behavior;
- `skip`: stale, already fixed, opinion-only or too broad;
- `question`: needs a product decision;
- `external`: CI or dependency issue for a separate workflow.

Record the rationale for every non-`apply` target. Ask before making broad changes.

## 4. Fix as focused commits

For each `apply` target: make the minimal fix, run the checks for the touched scope, stage only those files, review the staged diff and commit with a Conventional Commit message. Track `<thread or comment id> -> <commit sha>`. Do not create empty commits.

## 5. Push once

```bash
git status --short --branch
git push
```

Never force-push. Capture each commit URL with `gh browse <sha> --no-browser`.

## 6. Reply and resolve

For each fixed target, reply on the existing conversation with `Fixed in <commit-url>.`:

```bash
gh api -X POST repos/<owner>/<repo>/pulls/<number>/comments/<comment-database-id>/replies -f body='Fixed in <commit-url>.'
```

Do not use GraphQL `addPullRequestReviewThreadReply`; it creates pending review replies. Then resolve only the fixed review threads:

```bash
gh api graphql -f query='mutation($threadId:ID!) { resolveReviewThread(input: { threadId: $threadId }) { thread { id isResolved } } }' -F threadId='<thread-id>'
```

Standalone comments get a reply but nothing is resolved. Do not reply before the commit is pushed. Leave false positives open; add a short `False positive: <reason>.` reply only when it helps. Keep `skip`, `question` and `external` rationale in the terminal unless asked.

## 7. Report

PR URL; fixed targets with commit URLs and resolution status; false positives with reasons; skipped, question and external targets; push status; checks run; final `git status --short --branch`.
