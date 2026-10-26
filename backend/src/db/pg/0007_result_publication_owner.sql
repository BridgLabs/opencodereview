-- A result publication records the instance posting it, as a review post does, so only
-- publications whose instance stopped are taken over and posted again.
ALTER TABLE review_results ADD COLUMN owner_instance TEXT COLLATE "C";
