-- Merged pull requests' frozen code is purged as soon as the merge is confirmed. The old default
-- kept it a day, which let active repositories fill the archive store; move installs still on it.
UPDATE settings SET value = '0' WHERE key = 'review_archive_retention_days' AND value = '1';
