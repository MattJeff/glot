<?php
/**
 * Pure helpers (no WordPress call): settings validation, glot.js attributes, hreflang links. Covered by
 * tests/HelpersTest.php.
 *
 * @package glot
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

/** Switcher positions glot.js knows (`data-position`); '' = the project's setting in glot. */
const GLOT_POSITIONS = array( '', 'bottom-right', 'bottom-left', 'top-right', 'top-left', 'none' );

/** Defaults of the `glot_settings` option. */
function glot_defaults() {
	return array(
		'project'  => '',
		'key'      => '',
		'position' => '',
		'seo'      => 'off',
		'server'   => 'https://api.glotlang.com',
		'script'   => GLOT_SCRIPT_URL,
	);
}

/** An https URL without trailing slash, or the fallback. */
function glot_https_url( $u, $fallback ) {
	$u = rtrim( trim( (string) $u ), '/' );
	return preg_match( '#^https://[^\s/?\#]+(/[^\s?\#]*)?$#', $u ) ? $u : $fallback;
}

/** Validate the raw settings form; unknown fields are dropped, invalid values fall back to the defaults. */
function glot_sanitize_settings( $in ) {
	$d   = glot_defaults();
	$in  = is_array( $in ) ? $in : array();
	$get = function ( $k ) use ( $in ) {
		return isset( $in[ $k ] ) && is_scalar( $in[ $k ] ) ? trim( (string) $in[ $k ] ) : '';
	};
	return array(
		'project'  => preg_match( '/^[a-z0-9][a-z0-9-]{0,62}$/', $get( 'project' ) ) ? $get( 'project' ) : '',
		// Public key only: a secret (glot_…) key never belongs in a page.
		'key'      => preg_match( '/^pk_[A-Za-z0-9_-]{8,128}$/', $get( 'key' ) ) ? $get( 'key' ) : '',
		'position' => in_array( $get( 'position' ), GLOT_POSITIONS, true ) ? $get( 'position' ) : '',
		'seo'      => 'on' === $get( 'seo' ) ? 'on' : 'off',
		'server'   => glot_https_url( $get( 'server' ), $d['server'] ),
		'script'   => glot_https_url( $get( 'script' ), $d['script'] ),
	);
}

/**
 * Attributes of the glot.js tag (packages/edge/src/website.global.ts), or null before a project is set.
 * `$placed`: the page has a `[data-glot-switcher]` placeholder (shortcode, widget, block): the menu goes there,
 * inline, unless the switcher is turned off.
 */
function glot_script_attributes( $s, $placed = false ) {
	if ( '' === $s['project'] ) {
		return null;
	}
	$a = array(
		'src'          => $s['script'],
		'async'        => true,
		'data-project' => $s['project'],
		'data-server'  => $s['server'],
	);
	if ( '' !== $s['key'] ) {
		$a['data-key'] = $s['key'];
	}
	if ( 'none' === $s['position'] ) {
		$a['data-switcher'] = 'none';
	} elseif ( $placed ) {
		$a['data-position'] = '[data-glot-switcher]'; // a selector, not a corner: glot.js puts the menu in place.
	} elseif ( '' !== $s['position'] ) {
		$a['data-position'] = $s['position'];
	}
	return $a;
}

/** `hreflang` of a `/edge/v1/{project}/status` body: `{locale: Edge base URL}` (SEO-ready languages), or null. */
function glot_status_hreflang( $json ) {
	$j = json_decode( (string) $json, true );
	if ( ! is_array( $j ) ) {
		return null;
	}
	$out = array();
	foreach ( isset( $j['hreflang'] ) && is_array( $j['hreflang'] ) ? $j['hreflang'] : array() as $l => $b ) {
		if ( is_string( $l ) && preg_match( '/^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})*$/', $l ) && is_string( $b ) && preg_match( '#^https?://[^\s"<>]+$#', $b ) ) {
			$out[ $l ] = rtrim( $b, '/' );
		}
	}
	return $out;
}

/**
 * hreflang links of `$url` (a source page): the Edge base it is under (longest match) is swapped for each
 * language's base, same path; `x-default` = the page itself. Empty when the page is under no base, or alone.
 */
function glot_hreflang_links( $url, $bases ) {
	$own = '';
	foreach ( $bases as $b ) {
		if ( ( $url === $b || 0 === strpos( $url, $b . '/' ) ) && strlen( $b ) > strlen( $own ) ) {
			$own = $b;
		}
	}
	if ( '' === $own || count( $bases ) < 2 ) {
		return array();
	}
	$rest  = substr( $url, strlen( $own ) );
	$rest  = '' === $rest ? '/' : $rest;
	$links = array();
	foreach ( $bases as $l => $b ) {
		$links[ $l ] = $b . $rest;
	}
	$links['x-default'] = $own . $rest;
	return $links;
}

/** The connectors API answer → [connector id, webhook URL] or an error message. */
function glot_parse_connect_response( $code, $body ) {
	$j = json_decode( (string) $body, true );
	if ( 201 === (int) $code && isset( $j['connector']['id'], $j['connector']['webhook_url'] ) ) {
		return array( (string) $j['connector']['id'], (string) $j['connector']['webhook_url'] );
	}
	if ( isset( $j['error']['message'] ) && is_string( $j['error']['message'] ) ) {
		return $j['error']['message'];
	}
	return 'HTTP ' . (int) $code;
}
