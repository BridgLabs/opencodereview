### Fixed

- Read a review whose final JSON has an unescaped `"` inside a string, as in a title or quoted code
  like `status === "done"`, instead of failing it with "Review final response is not valid
  structured JSON (expected `,` or `}`)". The quote is escaped and the report validated as usual,
  so quotes still have to match the frozen source exactly.
