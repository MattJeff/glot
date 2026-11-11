<?php
/**
 * Plugin Name:       glot
 * Plugin URI:        https://glotlang.com/docs/integrations#wordpress-plugin
 * Description:       Translate your site with glot: one script tag, a language switcher (shortcode, widget, block, Elementor), hreflang to glot Edge, and an optional link to the glot WordPress connector.
 * Version:           0.1.0
 * Requires at least: 6.1
 * Requires PHP:      7.4
 * Author:            glot
 * Author URI:        https://glotlang.com
 * License:           GPL-2.0-or-later
 * License URI:       https://www.gnu.org/licenses/gpl-2.0.html
 * Text Domain:       glot
 * Domain Path:       /languages
 *
 * @package glot
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

define( 'GLOT_VERSION', '0.1.0' );
define( 'GLOT_SCRIPT_URL', 'https://app.glotlang.com/widget/glot.js' );
define( 'GLOT_DIR', plugin_dir_path( __FILE__ ) );

require_once GLOT_DIR . 'includes/helpers.php';

/** Current settings, validated. */
function glot_settings() {
	return wp_parse_args( (array) get_option( 'glot_settings', array() ), glot_defaults() );
}

/** The language switcher placeholder: glot.js puts its menu into the first `[data-glot-switcher]` of the page. */
function glot_switcher_html() {
	$GLOBALS['glot_placed'] = true; // read by the script tag, printed in the footer (after the content).
	return '<div class="glot-switcher" data-glot-switcher></div>';
}

add_action(
	'init',
	function () {
		load_plugin_textdomain( 'glot', false, dirname( plugin_basename( __FILE__ ) ) . '/languages' );
		add_shortcode( 'glot_switcher', 'glot_switcher_html' );
		register_block_type( GLOT_DIR . 'blocks/switcher' );
		wp_set_script_translations( 'glot-switcher-editor-script', 'glot', GLOT_DIR . 'languages' );
	}
);

add_action(
	'widgets_init',
	function () {
		require_once GLOT_DIR . 'includes/class-glot-widget.php';
		register_widget( 'Glot_Widget' );
	}
);

add_action(
	'elementor/widgets/register',
	function ( $widgets_manager ) {
		require_once GLOT_DIR . 'includes/class-glot-elementor-widget.php';
		$widgets_manager->register( new Glot_Elementor_Widget() );
	}
);

/* ------------------------------------------------------------------ Front end */

add_action(
	'wp_head',
	function () {
		$s = glot_settings();
		if ( '' !== $s['project'] && 'on' === $s['seo'] ) {
			glot_print_hreflang( $s );
		}
	},
	1
);

// In the footer: by then we know whether the page placed a switcher. glot.js is async and waits for the DOM anyway.
add_action(
	'wp_footer',
	function () {
		$a = glot_script_attributes( glot_settings(), ! empty( $GLOBALS['glot_placed'] ) );
		if ( $a ) {
			wp_print_script_tag( $a );
		}
	}
);

/**
 * The Edge bases of the project's SEO-ready languages (`hreflang` of `/edge/v1/{project}/status`: verified
 * domains, ready languages only), cached 1 h. The only request the plugin makes from the front end, in SEO mode.
 */
function glot_bases( $s ) {
	$cache = 'glot_bases_' . md5( $s['server'] . $s['project'] );
	$bases = get_transient( $cache );
	if ( is_array( $bases ) ) {
		return $bases;
	}
	$r     = wp_remote_get( $s['server'] . '/edge/v1/' . rawurlencode( $s['project'] ) . '/status', array( 'timeout' => 3 ) );
	$bases = is_wp_error( $r ) || 200 !== wp_remote_retrieve_response_code( $r ) ? null : glot_status_hreflang( wp_remote_retrieve_body( $r ) );
	if ( null === $bases ) {
		$bases = (array) get_option( 'glot_bases_last', array() ); // glot unreachable: last known, retried in 5 min.
		set_transient( $cache, $bases, 5 * MINUTE_IN_SECONDS );
		return $bases;
	}
	update_option( 'glot_bases_last', $bases, false );
	set_transient( $cache, $bases, HOUR_IN_SECONDS );
	return $bases;
}

/** WooCommerce pages that hold a customer's session or data: no hreflang there. */
function glot_is_private_page() {
	return function_exists( 'is_woocommerce' ) && ( is_cart() || is_checkout() || is_account_page() );
}

/** hreflang links from this page to its glot Edge URLs, in the HTML crawlers read (glot.js then adds none). */
function glot_print_hreflang( $s ) {
	if ( is_404() || is_search() || is_preview() || glot_is_private_page() ) {
		return;
	}
	$home = wp_parse_url( home_url( '/' ) );
	$uri  = isset( $_SERVER['REQUEST_URI'] ) ? wp_unslash( $_SERVER['REQUEST_URI'] ) : '/'; // phpcs:ignore WordPress.Security.ValidatedSanitizedInput.InputNotSanitized -- path only, escaped on output.
	// Host from the settings, never from the request (Host header), so a cached page can't carry a forged host.
	$url = $home['scheme'] . '://' . $home['host'] . ( isset( $home['port'] ) ? ':' . $home['port'] : '' ) . (string) wp_parse_url( $uri, PHP_URL_PATH );
	foreach ( glot_hreflang_links( $url, glot_bases( $s ) ) as $l => $u ) {
		printf( '<link rel="alternate" hreflang="%s" href="%s">' . "\n", esc_attr( $l ), esc_url( $u ) );
	}
}

/*
 * WooCommerce: customer data is never translated nor reported. Form values are already left alone by glot.js;
 * the addresses and customer details WooCommerce prints are wrapped in `data-glot-user`.
 */
const GLOT_WC_USER_TEMPLATES = array( 'order/order-details-customer.php', 'myaccount/my-address.php' );
add_action(
	'woocommerce_before_template_part',
	function ( $template ) {
		if ( in_array( $template, GLOT_WC_USER_TEMPLATES, true ) ) {
			echo '<div data-glot-user>';
		}
	}
);
add_action(
	'woocommerce_after_template_part',
	function ( $template ) {
		if ( in_array( $template, GLOT_WC_USER_TEMPLATES, true ) ) {
			echo '</div>';
		}
	}
);

/* ------------------------------------------------------------------ Connector webhook */

/** When linked to the glot connector: tell glot a post changed (WordPress core sends no webhooks). */
function glot_ping( $what ) {
	$c = get_option( 'glot_connection' );
	if ( empty( $c['webhook'] ) ) {
		return;
	}
	wp_remote_post(
		$c['webhook'],
		array(
			'blocking' => false,
			'timeout'  => 2,
			'headers'  => array( 'Content-Type' => 'application/json' ),
			'body'     => wp_json_encode( $what ),
		)
	);
}

add_action(
	'save_post',
	function ( $id, $post ) {
		if ( wp_is_post_revision( $id ) || wp_is_post_autosave( $id ) || ! in_array( $post->post_type, array( 'post', 'page' ), true ) || 'auto-draft' === $post->post_status ) {
			return;
		}
		glot_ping( array( 'event' => 'save_post', 'id' => (int) $id ) );
	},
	10,
	2
);
add_action(
	'edited_term',
	function ( $id, $tt, $taxonomy ) {
		if ( in_array( $taxonomy, array( 'category', 'post_tag' ), true ) ) {
			glot_ping( array( 'event' => 'edited_term', 'id' => (int) $id ) );
		}
	},
	10,
	3
);

if ( is_admin() ) {
	require_once GLOT_DIR . 'includes/admin.php';
}
