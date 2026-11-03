### Fixed

- Post a new finding even when an earlier OpenCodeReview comment sits on the same line. Before,
  any finding on an already commented line was held back as "already posted".
- Recognize a finding that a later push moved to another line as already posted, by its title in
  the same file, instead of posting it again.

### Security

- Redact credential-shaped text (GitHub, GitLab, AWS, Anthropic, OpenAI, Slack, Stripe, Google
  and npm tokens, private key blocks and URL passwords) from published findings, questions and
  suggestions.
- Neutralize Markdown images and `@` mentions in published review text, so a comment never makes
  GitHub fetch a URL chosen by the model or notify a user or team.
