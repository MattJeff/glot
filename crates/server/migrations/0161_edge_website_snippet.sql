-- The universal website tag (`<script src=".../widget/glot.js" data-project data-key>`, site builders: Wix,
-- Squarespace, Bubble…): translates the public pages in the browser from the project's TM, no proxy, no DNS.
--   on              = the tag is active (off: it does nothing, the page stays as published — a kill switch);
--   auto_translate  = new segments it discovers are machine-translated and served; false = they wait for a review;
--   position        = switcher placement: bottom-right | bottom-left | top-right | top-left | none, or a CSS
--                     selector the switcher is appended to;
--   locales         = languages offered in the switcher (empty = every target locale).
ALTER TABLE edge_settings
  ADD COLUMN website_snippet jsonb NOT NULL
    DEFAULT '{"on": false, "auto_translate": true, "position": "bottom-right", "locales": []}';
