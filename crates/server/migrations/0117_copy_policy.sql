-- Copy policy of a project (glot_core::policy::PolicyConfig: rule levels, banned terms per market, extra
-- wordings), applied to every generated / personalized copy variant. '{}' = defaults.
ALTER TABLE projects ADD copy_policy jsonb NOT NULL DEFAULT '{}';
