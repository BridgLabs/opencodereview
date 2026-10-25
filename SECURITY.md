# Security policy

## Supported versions

OpenCodeReview is released from `main`. Security fixes are made on `main` and included in the next release; only the latest release receives them.

| Version | Supported |
| --- | --- |
| Latest release and `main` | Yes |
| Older releases | No |

## Reporting a vulnerability

Please do not open a public issue, discussion or pull request for a security problem.

Report it privately through GitHub's private vulnerability reporting: open the repository's **Security** tab and choose **Report a vulnerability**, or go to <https://github.com/BridgLabs/opencodereview/security/advisories/new>.

Include:

- a description of the issue and its impact,
- steps to reproduce or a proof of concept,
- the affected version or commit,
- any suggested fix.

You should receive an acknowledgement within a few days. We will keep you informed while we investigate, agree on a disclosure date with you, and credit you in the advisory unless you prefer otherwise.

## Scope

Areas where reports are especially valuable:

- escaping the review runner sandbox, or a runner reaching hosts other than the AI providers;
- exposure of GitHub, Claude or ChatGPT credentials (logs, API responses, the browser, review output);
- prompt injection from pull request content that leads to actions outside the review;
- API authentication or origin-check bypasses.

Known limitations that are documented and not vulnerabilities on their own:

- The API token is embedded in the UI, so anyone who can open the UI can use the API. OpenCodeReview is meant to run on a trusted network or behind an authenticated reverse proxy.
- Pull request content is sent to the AI provider you signed in with.
