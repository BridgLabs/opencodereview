-- SQLite's JSON functions, with the semantics the queries were written against. The JSON stays in
-- TEXT columns; invalid JSON reads as absent, as the queries' json_valid guards made it.

-- json_array_length(value, '$.member'): the array's length, 0 for any other JSON value, NULL when
-- the member is absent or the text is not JSON. jsonb_array_length raises on non-arrays, and
-- Postgres does not promise to evaluate an AND guard first, so the check lives in here.
CREATE FUNCTION json_member_array_length(value TEXT, member TEXT) RETURNS BIGINT
  LANGUAGE sql IMMUTABLE PARALLEL SAFE
  AS $$
    SELECT CASE
      WHEN try_jsonb(value) -> member IS NULL THEN NULL
      WHEN jsonb_typeof(try_jsonb(value) -> member) = 'array' THEN jsonb_array_length(try_jsonb(value) -> member)
      ELSE 0
    END
  $$;

-- json_extract(value, '$.member') = 1: SQLite returns JSON true as 1, so either matched.
CREATE FUNCTION json_member_is_true(value TEXT, member TEXT) RETURNS BOOLEAN
  LANGUAGE sql IMMUTABLE PARALLEL SAFE
  AS $$ SELECT COALESCE(try_jsonb(value) -> member IN ('true'::jsonb, '1'::jsonb), false) $$;

-- json_type(value, '$.member'): the member's JSON type ('array', 'object', …), NULL when absent.
CREATE FUNCTION json_member_type(value TEXT, member TEXT) RETURNS TEXT
  LANGUAGE sql IMMUTABLE PARALLEL SAFE
  AS $$ SELECT jsonb_typeof(try_jsonb(value) -> member) $$;

-- json_remove(value, '$.member'), keeping invalid JSON as it is rather than failing the write.
CREATE FUNCTION json_without_member(value TEXT, member TEXT) RETURNS TEXT
  LANGUAGE sql IMMUTABLE PARALLEL SAFE
  AS $$ SELECT COALESCE((try_jsonb(value) - member)::text, value) $$;
