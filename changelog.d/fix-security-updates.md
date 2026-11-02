### Security

- The web image moves to nginx 1.30 and takes Alpine's security updates at build time, fixing a critical
  brotli vulnerability and high ones in curl, OpenSSL, expat, pcre2 and other base packages. The server
  and runner images take Debian's security updates at build time too.
- Update pulldown-cmark to 0.13.6 (RUSTSEC-2026-0433). OpenCodeReview did not enable the affected
  wikilink parsing.
