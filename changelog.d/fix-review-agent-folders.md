### Fixed

- Committed agent instructions, skills and scripts in folders such as `.claude`, `.agents`,
  `.gemini` and `.cursor` are now reviewed. The AI reads them only from the frozen archive, never
  from its workspace, so they cannot act as its configuration. Their settings, MCP servers,
  sign-ins and session state stay excluded, along with `.mcp.json`, `.git`, `.ssh` and `.gnupg`.
