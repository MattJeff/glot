-- Signup anti-abuse (auth.rs): client IP of the signup (X-Forwarded-For set by Caddy), for the per-IP signup limit.
-- NULL for accounts created before, or without a proxy in front (direct/dev).
ALTER TABLE users ADD COLUMN signup_ip text;
CREATE INDEX users_signup_ip ON users (signup_ip, created_at) WHERE signup_ip IS NOT NULL;
