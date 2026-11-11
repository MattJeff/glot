<?php
/**
 * Removes everything the plugin stored, and revokes the application password it created.
 *
 * @package glot
 */

if ( ! defined( 'WP_UNINSTALL_PLUGIN' ) ) {
	exit;
}

$glot_c = get_option( 'glot_connection' );
if ( ! empty( $glot_c['uuid'] ) && class_exists( 'WP_Application_Passwords' ) ) {
	WP_Application_Passwords::delete_application_password( (int) $glot_c['user'], $glot_c['uuid'] );
}
delete_option( 'glot_settings' );
delete_option( 'glot_connection' );
delete_option( 'glot_bases_last' );
global $wpdb;
// phpcs:ignore WordPress.DB.DirectDatabaseQuery -- transients named glot_bases_<hash>, no API lists them.
$wpdb->query( "DELETE FROM {$wpdb->options} WHERE option_name LIKE '\\_transient\\_glot\\_bases\\_%' OR option_name LIKE '\\_transient\\_timeout\\_glot\\_bases\\_%'" );
