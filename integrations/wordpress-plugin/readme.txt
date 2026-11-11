=== glot ===
Contributors: glot
Tags: translation, multilingual, language switcher, hreflang, woocommerce
Requires at least: 6.1
Tested up to: 6.8
Requires PHP: 7.4
Stable tag: 0.1.0
License: GPLv2 or later
License URI: https://www.gnu.org/licenses/gpl-2.0.html

Translate your WordPress site with glot: one script tag, a language switcher, hreflang to glot Edge, and an optional sync of posts and pages.

== Description ==

glot translates your site with your own model, glossary and validators. This plugin puts glot on a WordPress site without touching your theme.

* **Script tag**: adds the glot website tag (`glot.js`) to every page, set up with your project and public key. Languages are chosen in glot (glot Edge → Website tag).
* **Language switcher**: the `[glot_switcher]` shortcode, a widget, a block (glot language switcher) and an Elementor widget. Without one of them, the menu sits in a corner of the page (or nowhere, if you build your own and call `glot.setLocale("es")`).
* **SEO with glot Edge**: when glot Edge serves translated pages on their own URLs (`es.example.com` or `example.com/es/`), the plugin adds the hreflang links from your pages to them, for languages glot reports as ready.
* **WooCommerce**: form values are never translated nor sent to glot. Customer details and addresses (order details, My account addresses) are marked as user content. Cart, checkout and account pages get no hreflang.
* **Sync posts and pages (optional)**: "Connect to glot" links the site to the glot WordPress connector, which writes translations into Polylang Pro or WPML. The plugin then tells glot when a post, page, category or tag is saved.

= Why hreflang to glot Edge, not translated pages served by WordPress =

Serving `/es/…` from WordPress itself would mean a request from PHP to glot on every page view, a second copy of the page in your cache plugins, and a slow page each time glot is slow. glot Edge already serves translated HTML (first byte translated, canonical, hreflang, sitemaps) from a subdomain or, through a Cloudflare Worker, from a subdirectory of your own domain. The plugin's job is to point search engines from your WordPress pages to those URLs.

= External services =

This plugin connects to glot (https://glotlang.com), the translation service it is made for. What glot receives and keeps: https://glotlang.com/docs/edge and https://glotlang.com/docs/integrations.

* **Visitors' browsers** load the glot script from the script URL (by default `https://app.glotlang.com/widget/glot.js`), which fetches your project's translations from the glot server and sends it public text it has no translation for yet (never form values, personal data, or pages seen with a session cookie). No cookie of your site is read or sent. The script stores the visitor's language choice in localStorage.
* **Your server**, only when SEO mode is on: `GET {server}/edge/v1/{project}/status` (the glot Edge URLs of your ready languages), at most once an hour. Nothing about visitors is sent.
* **Your server**, only when you click "Connect to glot": one request to `{server}/api/projects/{project}/connectors` with your glot API token (not stored), your site URL, your username and a new application password. After that, a request to the connector's webhook URL each time a post, page, category or tag is saved (post or term id only).

== Installation ==

1. Upload the `glot` folder to `/wp-content/plugins/`, or install the zip from Plugins → Add New → Upload Plugin.
2. Activate it, then open Settings → glot.
3. Fill in your project slug and public key (`pk_…`, from glot Edge → Website tag). Save. In glot, turn the website tag on and pick the languages.
4. Optional: put the `[glot_switcher]` shortcode, the widget or the block where you want the menu.
5. Optional (SEO): set up a domain in glot (glot Edge → Setup), then turn on "hreflang to glot Edge".
6. Optional (sync): install Polylang Pro or WPML, paste a glot API token and click "Connect to glot".

== Frequently Asked Questions ==

= Do I need a multilingual plugin? =

No, for translation in the browser and with glot Edge. Yes (Polylang Pro or WPML) only for the optional sync, which writes translations into WordPress posts.

= Is my secret API key exposed? =

No. Only a public `pk_` key goes into your pages; the settings refuse anything else. The `glot_…` token used to connect is sent once to glot and never saved.

= Can I have several switchers on a page? =

The script fills the first one it finds. Use one per page.

== Changelog ==

= 0.1.0 =
* First version: script tag, switcher (shortcode, widget, block, Elementor), hreflang to glot Edge, WooCommerce customer data protection, connector link.
