#!/bin/sh
# Runs once, when the Postgres volume is first initialized, as the owner role (POSTGRES_USER),
# which runs the server's migrations. The server reads and writes rows as ocr_app and serves read
# paths as ocr_ro; neither can change the schema.
set -eu
psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" \
  -v app_password="$(cat /secret/app-password)" \
  -v ro_password="$(cat /secret/ro-password)" <<'SQL'
CREATE ROLE ocr_app LOGIN PASSWORD :'app_password';
CREATE ROLE ocr_ro LOGIN PASSWORD :'ro_password';
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
-- Every table and sequence the owner creates from now on, migrations included.
ALTER DEFAULT PRIVILEGES GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO ocr_app;
ALTER DEFAULT PRIVILEGES GRANT USAGE, SELECT ON SEQUENCES TO ocr_app;
ALTER DEFAULT PRIVILEGES GRANT SELECT ON TABLES TO ocr_ro;
SQL
