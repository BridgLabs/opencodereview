### Fixed

- Let ChatGPT (Codex) reviews read the pull request's files again. Before, every shell command
  Codex ran failed with "code-mode host is disabled", so each review failed with every assigned file
  reported as `file_unavailable`.
