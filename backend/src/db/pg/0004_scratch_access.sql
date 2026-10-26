-- The server may run as a role other than the schema owner (Compose uses ocr_app). Table
-- privileges still decide what it can do in `scratch`; this only lets it name the schema.
GRANT USAGE ON SCHEMA scratch TO PUBLIC;
