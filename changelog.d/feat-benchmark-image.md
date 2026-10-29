### Added

- Benchmark reviews have a time budget. Close to the deadline they skip the lens pass, output
  repairs and final integration batches instead of running out of time, keeping the findings
  already validated. Product reviews are unchanged.
- A benchmark review the deadline still cancels reports the findings its finished chunks
  validated instead of none.
- The Martian benchmark adapter can split its pull requests across several processes, one per
  account, without them overwriting each other's results.
- The AACR-Bench adapter can split its instances across accounts the same way.
- Benchmark exports include the suggestions a review lists in its summary, after its inline
  findings.
- A benchmark pull request whose diff holds a file in another encoding is reviewed instead of
  failing before the review: the benchmark reads the diff as the product does.
- A benchmark repository cached with only the commits it needs (a shallow cache) checks out its
  pull requests instead of failing to read their files.
- The Markdown parser is updated to 0.13.5 for RUSTSEC-2026-0414. OpenCodeReview never enables the
  affected wikilinks option, so reviews were not exposed.
