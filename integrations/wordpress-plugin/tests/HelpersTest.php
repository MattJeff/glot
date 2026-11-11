<?php
// Pure helpers, without WordPress: `composer install && composer test`.
use PHPUnit\Framework\TestCase;

define( 'ABSPATH', __DIR__ );
define( 'GLOT_SCRIPT_URL', 'https://app.glotlang.com/widget/glot.js' );
require_once __DIR__ . '/../includes/helpers.php';

final class HelpersTest extends TestCase {
	public function test_sanitize_keeps_valid_and_drops_invalid() {
		$s = glot_sanitize_settings(
			array(
				'project'  => 'my-site',
				'key'      => 'glot_secret_123456789', // a secret key is refused.
				'server'   => 'http://insecure.example',
				'position' => 'middle',
				'seo'      => 'on',
				'extra'    => 'x',
			)
		);
		$this->assertSame( 'my-site', $s['project'] );
		$this->assertSame( '', $s['key'] );
		$this->assertSame( 'https://api.glotlang.com', $s['server'] );
		$this->assertSame( '', $s['position'] );
		$this->assertSame( 'on', $s['seo'] );
		$this->assertArrayNotHasKey( 'extra', $s );
		$this->assertSame( 'pk_live_abcdef12', glot_sanitize_settings( array( 'key' => 'pk_live_abcdef12' ) )['key'] );
		$this->assertSame( '', glot_sanitize_settings( array( 'project' => 'Bad Slug"><script>' ) )['project'] );
		$this->assertSame( 'top-left', glot_sanitize_settings( array( 'position' => 'top-left' ) )['position'] );
	}

	public function test_script_attributes() {
		$this->assertNull( glot_script_attributes( glot_defaults() ) );
		$s = glot_sanitize_settings( array( 'project' => 'site', 'key' => 'pk_abcdefgh' ) );
		$a = glot_script_attributes( $s );
		$this->assertSame( GLOT_SCRIPT_URL, $a['src'] );
		$this->assertTrue( $a['async'] );
		$this->assertSame( 'site', $a['data-project'] );
		$this->assertSame( 'pk_abcdefgh', $a['data-key'] );
		$this->assertArrayNotHasKey( 'data-position', $a ); // the project's setting in glot.
		$this->assertSame( '[data-glot-switcher]', glot_script_attributes( $s, true )['data-position'] );
		$s['position'] = 'bottom-left';
		$this->assertSame( 'bottom-left', glot_script_attributes( $s )['data-position'] );
		$s['position'] = 'none';
		$a = glot_script_attributes( $s, true );
		$this->assertSame( 'none', $a['data-switcher'] );
		$this->assertArrayNotHasKey( 'data-position', $a );
	}

	public function test_status_hreflang() {
		$this->assertSame(
			array( 'en' => 'https://x.io', 'es' => 'https://es.x.io' ),
			glot_status_hreflang( '{"enabled":true,"hreflang":{"en":"https://x.io/","es":"https://es.x.io","bad code":"https://y","de":"javascript:alert(1)"}}' )
		);
		$this->assertSame( array(), glot_status_hreflang( '{"enabled":true}' ) );
		$this->assertNull( glot_status_hreflang( '<html>' ) );
	}

	public function test_hreflang_links() {
		$sub = array( 'en' => 'https://x.io', 'es' => 'https://es.x.io' );
		$this->assertSame(
			array( 'en' => 'https://x.io/pricing/', 'es' => 'https://es.x.io/pricing/', 'x-default' => 'https://x.io/pricing/' ),
			glot_hreflang_links( 'https://x.io/pricing/', $sub )
		);
		$this->assertSame( 'https://es.x.io/', glot_hreflang_links( 'https://x.io', $sub )['es'] );
		$dir = array( 'en' => 'https://x.io', 'es' => 'https://x.io/es' );
		$this->assertSame( 'https://x.io/es/blog/post/', glot_hreflang_links( 'https://x.io/blog/post/', $dir )['es'] );
		$this->assertSame( array(), glot_hreflang_links( 'https://www.x.io/', $sub ) ); // not under any base.
		$this->assertSame( array(), glot_hreflang_links( 'https://x.iox/', $sub ) );
		$this->assertSame( array(), glot_hreflang_links( 'https://x.io/', array( 'en' => 'https://x.io' ) ) ); // alone.
	}

	public function test_connect_response() {
		$this->assertSame( array( 'c1', 'https://api.glotlang.com/api/connectors/c1/webhook?token=t' ), glot_parse_connect_response( 201, '{"connector":{"id":"c1","webhook_url":"https://api.glotlang.com/api/connectors/c1/webhook?token=t"}}' ) );
		$this->assertSame( 'forbidden', glot_parse_connect_response( 403, '{"error":{"code":"forbidden","message":"forbidden"}}' ) );
		$this->assertSame( 'HTTP 502', glot_parse_connect_response( 502, '<html>' ) );
	}
}
