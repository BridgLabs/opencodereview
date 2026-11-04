### Fixed

- Publish reviews whose findings hold very long words or tokens without stalling. Redacting
  credentials scanned a token again from each of its characters, so a 30,000-character token took
  minutes.
