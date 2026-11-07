-- Message syntax of the project's catalogs (glot.toml `syntax`): drives server-side validation and LLM prompts.
-- syntax_locked = chosen in the dashboard; `glot push` then only changes it with --set-syntax.
ALTER TABLE projects
    ADD COLUMN syntax text NOT NULL DEFAULT 'icu'
        CHECK (syntax IN ('icu', 'i18next', 'mf2', 'printf', 'rails', 'vue', 'laravel', 'template', 'fluent', 'plain', 'symfony', 'java')),
    ADD COLUMN syntax_locked boolean NOT NULL DEFAULT false;
