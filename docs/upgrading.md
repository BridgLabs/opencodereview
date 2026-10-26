# Upgrading

## From SQLite to Postgres

OpenCodeReview stored its data in a SQLite file (`/data/opencodereview.sqlite`) and the review archive index in `/data/review-archives/index.sqlite`. It now stores both in Postgres, which Docker Compose runs as the `postgres` service. An existing install moves its data once, with the server stopped. The archives themselves, sign-ins and the Git cache stay where they are, in the `ocr-data` volume.

Until the import has run, the server refuses to open the review archives. Their new index is still empty then, and opening it would treat every archive as unreferenced.

1. Stop OpenCodeReview and keep a copy of the SQLite file:

   ```bash
   docker compose stop
   docker compose run --rm --no-deps -T --entrypoint cat server /data/opencodereview.sqlite > opencodereview-backup.sqlite
   ```

2. Update to this release and start only the database. The first start generates the database passwords:

   ```bash
   git pull
   docker compose build
   docker compose up -d postgres
   ```

3. Import:

   ```bash
   docker compose run --rm server ocr-server import-sqlite /data/opencodereview.sqlite
   ```

   The import reads a snapshot of the file and never changes it. It also imports `/data/review-archives/index.sqlite` when present, then renames it `index.sqlite.imported`. It prints a JSON report:

   | Field | What it lists |
   | --- | --- |
   | `tables` | Rows copied per table. |
   | `restoredRepositories` | Repositories referenced by history but no longer in the file. They come back marked removed. |
   | `clearedReferences` | References to rows that no longer exist, set to empty. |
   | `skippedOrphans` | Rows whose required parent no longer exists, left out. |
   | `repairedText` | Text with bytes Postgres cannot store, repaired. |

   The import refuses to run while a server is running, and it refuses a database that already has data, so it can only happen once. A file from an older version is brought up to date on the snapshot first. A file from a newer version is refused.

4. Start everything:

   ```bash
   docker compose up -d
   ```

Once the reviews, settings and accounts show up in the UI, the old files can go:

```bash
docker compose run --rm --no-deps --entrypoint rm server /data/opencodereview.sqlite /data/review-archives/index.sqlite.imported
rm opencodereview-backup.sqlite
```

## Running on your own Postgres

Compose's `postgres` service is the supported setup. To use another Postgres (16 or later), give the server these variables. Each one names a file holding a connection URL, so no credential appears in the environment or in `docker inspect`:

| Variable | Role it should connect as |
| --- | --- |
| `OCR_DATABASE_URL_FILE` | Reads and writes rows (`ocr_app` in Compose). |
| `OCR_DATABASE_RO_URL_FILE` | Reads only (`ocr_ro`). Optional; defaults to the first URL with read-only transactions. |
| `OCR_DATABASE_OWNER_URL_FILE` | Owns the schema and runs migrations (`ocr`). Optional; defaults to the first URL. |

[`docker/postgres/init-roles.sh`](../docker/postgres/init-roles.sh) shows the grants these roles need. A database outside the local network must be reached over TLS: add `sslmode=require` to the URL, and set `OCR_DATABASE_CA_FILE` when the server's certificate is not signed by a public authority.
