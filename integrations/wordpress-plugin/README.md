# glot for WordPress

WordPress plugin: glot script tag, language switcher (shortcode `[glot_switcher]`, widget, block, Elementor widget), hreflang to glot Edge, WooCommerce customer data protection, and "Connect to glot" for the server WordPress connector. Full guide: [docs/integrations#wordpress-plugin](../../web/content/docs/integrations.md#wordpress-plugin). WordPress.org readme: [`readme.txt`](readme.txt).

```sh
composer install && composer test                    # PHPUnit: settings validation, hreflang URLs, script tag, connector answer
zip -r glot.zip . -x 'vendor/*' 'tests/*' 'composer.*' '.gitignore' 'README.md'   # installable zip (Plugins → Add New → Upload)
```

Translations: `languages/glot.pot` (template), `glot-fr_FR.po/.mo`, and the block's JSON. Regenerate the template with `wp i18n make-pot . languages/glot.pot`, or `xgettext --language=PHP --keyword=__ --keyword=_e --keyword=esc_html__ --keyword=esc_html_e --add-comments=translators -o languages/glot.pot glot.php includes/*.php` then `xgettext --language=JavaScript --keyword=__ -j -o languages/glot.pot blocks/switcher/index.js`.
