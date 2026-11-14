-- Where a signup came from (utm_* of the ad or link that brought the visitor). No cookie, no identifier.
ALTER TABLE users ADD COLUMN acquisition jsonb;
