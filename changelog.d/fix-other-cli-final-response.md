### Fixed

- Accept reviews from ChatGPT when the model sends the report as a JSON string
  or the finding decisions as a bare or empty list. Before, those reviews failed with "Review final
  response requires JSON objects for report and findingDecisions".
