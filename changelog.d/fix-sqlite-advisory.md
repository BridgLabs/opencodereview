### Security

- Update the bundled SQLite used by `ocr-server import-sqlite` to a version without
  CVE-2026-119375 (RUSTSEC-2026-0459), which could expose page-cache memory when opening a
  database file together with a write-ahead log written by a less trusted party.
