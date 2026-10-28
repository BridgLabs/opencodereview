### Added

- Benchmark reviews have a time budget. Close to the deadline they skip the lens pass, output
  repairs and final integration batches instead of running out of time, keeping the findings
  already validated. Product reviews are unchanged.
- A benchmark review the deadline still cancels reports the findings its finished chunks
  validated instead of none.
- The Martian benchmark adapter can split its pull requests across several processes, one per
  account, without them overwriting each other's results.
