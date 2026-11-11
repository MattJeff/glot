-- Growth (team) free trial: offered once per account; set when a subscription with a trial (or on Growth) is applied.
ALTER TABLE billing_accounts ADD COLUMN trial_used_at timestamptz;
