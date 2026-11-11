<?php
/**
 * Settings page (Settings → glot) and the "Connect to glot" link to the server connector.
 *
 * @package glot
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

add_action(
	'admin_init',
	function () {
		register_setting(
			'glot',
			'glot_settings',
			array(
				'type'              => 'array',
				'sanitize_callback' => 'glot_sanitize_settings',
				'default'           => glot_defaults(),
			)
		);
	}
);

add_action(
	'admin_menu',
	function () {
		add_options_page( 'glot', 'glot', 'manage_options', 'glot', 'glot_settings_page' );
	}
);

add_filter(
	'plugin_action_links_' . plugin_basename( GLOT_DIR . 'glot.php' ),
	function ( $links ) {
		array_unshift( $links, '<a href="' . esc_url( admin_url( 'options-general.php?page=glot' ) ) . '">' . esc_html__( 'Settings', 'glot' ) . '</a>' );
		return $links;
	}
);

/** One text/select row of the settings table. */
function glot_field( $s, $name, $label, $help, $choices = null ) {
	$id = 'glot_' . $name;
	echo '<tr><th scope="row"><label for="' . esc_attr( $id ) . '">' . esc_html( $label ) . '</label></th><td>';
	if ( $choices ) {
		echo '<select id="' . esc_attr( $id ) . '" name="glot_settings[' . esc_attr( $name ) . ']">';
		foreach ( $choices as $v => $l ) {
			echo '<option value="' . esc_attr( $v ) . '"' . selected( $s[ $name ], $v, false ) . '>' . esc_html( $l ) . '</option>';
		}
		echo '</select>';
	} else {
		echo '<input type="text" class="regular-text" id="' . esc_attr( $id ) . '" name="glot_settings[' . esc_attr( $name ) . ']" value="' . esc_attr( $s[ $name ] ) . '">';
	}
	echo '<p class="description">' . wp_kses( $help, array( 'code' => array() ) ) . '</p></td></tr>';
}

function glot_settings_page() {
	if ( ! current_user_can( 'manage_options' ) ) {
		return;
	}
	$s = glot_settings();
	$c = get_option( 'glot_connection' );
	// Notices come from our own redirects (glot_notice = a fixed code, glot_error = the glot server's message).
	$notice = isset( $_GET['glot_notice'] ) ? sanitize_key( wp_unslash( $_GET['glot_notice'] ) ) : ''; // phpcs:ignore WordPress.Security.NonceVerification.Recommended -- display only.
	$error  = get_transient( 'glot_error_' . get_current_user_id() );
	delete_transient( 'glot_error_' . get_current_user_id() );
	?>
	<div class="wrap">
		<h1>glot</h1>
		<?php if ( $error ) : ?>
			<div class="notice notice-error"><p><?php echo esc_html( $error ); ?></p></div>
		<?php elseif ( 'connected' === $notice ) : ?>
			<div class="notice notice-success"><p><?php esc_html_e( 'Connected. glot now syncs your posts and pages, and is notified when you save one.', 'glot' ); ?></p></div>
		<?php elseif ( 'disconnected' === $notice ) : ?>
			<div class="notice notice-success"><p><?php esc_html_e( 'Disconnected. The application password was revoked. Delete the WordPress card in glot as well.', 'glot' ); ?></p></div>
		<?php endif; ?>

		<form method="post" action="options.php">
			<?php settings_fields( 'glot' ); ?>
			<table class="form-table" role="presentation">
				<?php
				glot_field( $s, 'project', __( 'Project', 'glot' ), __( 'Your glot project slug (dashboard URL: <code>/app/your-project</code>).', 'glot' ) );
				glot_field( $s, 'key', __( 'Public key', 'glot' ), __( 'Starts with <code>pk_</code> (glot Edge → Website tag). Never paste a secret <code>glot_…</code> key here: this one is printed in your pages.', 'glot' ) );
				glot_field(
					$s,
					'position',
					__( 'Language switcher', 'glot' ),
					__( 'A page with the <code>[glot_switcher]</code> shortcode, the widget or the block shows the menu there instead. Languages, and turning the tag on, are set in glot (glot Edge → Website tag).', 'glot' ),
					array(
						''             => __( 'As set in glot', 'glot' ),
						'bottom-right' => __( 'Floating, bottom right', 'glot' ),
						'bottom-left'  => __( 'Floating, bottom left', 'glot' ),
						'top-right'    => __( 'Floating, top right', 'glot' ),
						'top-left'     => __( 'Floating, top left', 'glot' ),
						'none'         => __( 'Hidden (my own switcher calls glot.setLocale)', 'glot' ),
					)
				);
				glot_field(
					$s,
					'seo',
					__( 'SEO (glot Edge)', 'glot' ),
					__( 'When glot Edge serves your languages on their own URLs (es.example.com or example.com/es/), adds hreflang links to them in every page, for the languages glot reports as ready. Set up the domain first (glot Edge → Setup).', 'glot' ),
					array(
						'off' => __( 'Off', 'glot' ),
						'on'  => __( 'hreflang to glot Edge', 'glot' ),
					)
				);
				glot_field( $s, 'server', __( 'glot server', 'glot' ), __( 'Change only for a self-hosted glot.', 'glot' ) );
				glot_field( $s, 'script', __( 'Script URL', 'glot' ), __( 'Change only for a self-hosted glot.', 'glot' ) );
				?>
			</table>
			<?php submit_button(); ?>
		</form>

		<h2><?php esc_html_e( 'Sync posts and pages (optional)', 'glot' ); ?></h2>
		<p><?php esc_html_e( 'The script above translates pages in the browser. To also write translations into WordPress (Polylang Pro or WPML), connect the glot WordPress connector: this creates an application password for your account and registers it in your glot project. Your site must be reachable from the internet, with pretty permalinks.', 'glot' ); ?></p>
		<?php if ( ! empty( $c['id'] ) ) : ?>
			<p><?php echo esc_html( sprintf( /* translators: %s: connector id */ __( 'Connected (connector %s).', 'glot' ), $c['id'] ) ); ?></p>
			<form method="post" action="<?php echo esc_url( admin_url( 'admin-post.php' ) ); ?>">
				<input type="hidden" name="action" value="glot_disconnect">
				<?php wp_nonce_field( 'glot_disconnect' ); ?>
				<?php submit_button( __( 'Disconnect', 'glot' ), 'secondary' ); ?>
			</form>
		<?php else : ?>
			<form method="post" action="<?php echo esc_url( admin_url( 'admin-post.php' ) ); ?>">
				<input type="hidden" name="action" value="glot_connect">
				<?php wp_nonce_field( 'glot_connect' ); ?>
				<p>
					<label for="glot_token"><?php esc_html_e( 'glot API token (used once, never stored)', 'glot' ); ?></label><br>
					<input type="password" class="regular-text" id="glot_token" name="glot_token" autocomplete="off" placeholder="glot_…">
				</p>
				<?php submit_button( __( 'Connect to glot', 'glot' ), 'primary', 'submit', true, '' === $s['project'] ? array( 'disabled' => 'disabled' ) : array() ); ?>
			</form>
		<?php endif; ?>
	</div>
	<?php
}

/** Back to the settings page with an error message (kept in a short transient, not in the URL). */
function glot_fail( $msg ) {
	set_transient( 'glot_error_' . get_current_user_id(), $msg, 60 );
	wp_safe_redirect( admin_url( 'options-general.php?page=glot' ) );
	exit;
}

add_action(
	'admin_post_glot_connect',
	function () {
		if ( ! current_user_can( 'manage_options' ) ) {
			wp_die( esc_html__( 'You are not allowed to do this.', 'glot' ), 403 );
		}
		check_admin_referer( 'glot_connect' );
		$s     = glot_settings();
		$token = isset( $_POST['glot_token'] ) ? trim( sanitize_text_field( wp_unslash( $_POST['glot_token'] ) ) ) : '';
		if ( '' === $s['project'] ) {
			glot_fail( __( 'Save your project slug first.', 'glot' ) );
		}
		if ( ! preg_match( '/^glot_[A-Za-z0-9_-]{8,}$/', $token ) ) {
			glot_fail( __( 'Paste a glot API token (glot_…) from your glot account settings.', 'glot' ) );
		}
		$user = wp_get_current_user();
		if ( ! wp_is_application_passwords_available_for_user( $user ) ) {
			glot_fail( __( 'Application passwords are disabled on this site (they need HTTPS, or a security plugin turned them off).', 'glot' ) );
		}
		$created = WP_Application_Passwords::create_new_application_password( $user->ID, array( 'name' => 'glot' ) );
		if ( is_wp_error( $created ) ) {
			glot_fail( $created->get_error_message() );
		}
		list( $password, $item ) = $created;
		$r = wp_remote_post(
			$s['server'] . '/api/projects/' . rawurlencode( $s['project'] ) . '/connectors',
			array(
				'timeout' => 60, // glot tests the credentials against this site before answering.
				'headers' => array(
					'Authorization' => 'Bearer ' . $token,
					'Content-Type'  => 'application/json',
				),
				'body'    => wp_json_encode(
					array(
						'kind'    => 'wordpress',
						'name'    => get_bloginfo( 'name' ),
						'secrets' => array(
							'url'          => home_url( '/' ),
							'username'     => $user->user_login,
							'app_password' => $password,
						),
					)
				),
			)
		);
		$res = is_wp_error( $r ) ? $r->get_error_message() : glot_parse_connect_response( wp_remote_retrieve_response_code( $r ), wp_remote_retrieve_body( $r ) );
		if ( ! is_array( $res ) ) {
			WP_Application_Passwords::delete_application_password( $user->ID, $item['uuid'] );
			/* translators: %s: error from the glot server */
			glot_fail( sprintf( __( 'glot refused the connection: %s', 'glot' ), $res ) );
		}
		update_option(
			'glot_connection',
			array(
				'id'      => $res[0],
				'webhook' => esc_url_raw( $res[1] ),
				'user'    => $user->ID,
				'uuid'    => $item['uuid'],
			),
			false
		);
		wp_safe_redirect( admin_url( 'options-general.php?page=glot&glot_notice=connected' ) );
		exit;
	}
);

add_action(
	'admin_post_glot_disconnect',
	function () {
		if ( ! current_user_can( 'manage_options' ) ) {
			wp_die( esc_html__( 'You are not allowed to do this.', 'glot' ), 403 );
		}
		check_admin_referer( 'glot_disconnect' );
		$c = get_option( 'glot_connection' );
		if ( ! empty( $c['uuid'] ) ) {
			WP_Application_Passwords::delete_application_password( (int) $c['user'], $c['uuid'] );
		}
		delete_option( 'glot_connection' );
		wp_safe_redirect( admin_url( 'options-general.php?page=glot&glot_notice=disconnected' ) );
		exit;
	}
);
