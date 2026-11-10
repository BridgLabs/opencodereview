### Added

- Review with OpenCode: sign in to the OpenCode console with a one-time code, like ChatGPT, or
  connect an OpenCode API key, which OpenCodeReview checks with OpenCode before saving it. The top bar
  lists OpenCode's models straight from opencode.ai, refreshed hourly: Go models on the Go plan and
  Zen models on the workspace credits, each with the efforts OpenCode offers for it, and only for
  the plans your OpenCode workspace has. Free models are left out. OpenCode accounts take part in failover like Claude and ChatGPT ones. Review runners run
  OpenCode 2.2.6 with a private server, no project configuration, plugins, MCP servers, skills, free
  models or web tools, and may reach `opencode.ai` only. Settings doesn't show OpenCode usage limits;
  check them in your OpenCode workspace, and turn off Go's Extra Usage there to keep Go reviews
  within the plan.

### Changed

- A review keeps the provider, model and effort chosen when it started. Switching the engine in
  the top bar no longer moves reviews that are already running; it applies to the next ones.
