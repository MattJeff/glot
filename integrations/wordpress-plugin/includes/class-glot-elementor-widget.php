<?php
/**
 * Elementor widget: the language switcher anywhere in an Elementor layout.
 * https://developers.elementor.com/docs/widgets/
 *
 * @package glot
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

class Glot_Elementor_Widget extends \Elementor\Widget_Base {
	public function get_name() {
		return 'glot_switcher';
	}

	public function get_title() {
		return __( 'glot language switcher', 'glot' );
	}

	public function get_icon() {
		return 'eicon-globe';
	}

	public function get_categories() {
		return array( 'general' );
	}

	public function get_keywords() {
		return array( 'language', 'translate', 'glot' );
	}

	protected function render() {
		echo glot_switcher_html(); // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- static markup.
	}
}
