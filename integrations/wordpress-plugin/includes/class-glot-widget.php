<?php
/**
 * Classic widget: the language switcher in a sidebar.
 *
 * @package glot
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

class Glot_Widget extends WP_Widget {
	public function __construct() {
		parent::__construct( 'glot_switcher', __( 'glot language switcher', 'glot' ), array( 'description' => __( 'Lets visitors pick a language.', 'glot' ) ) );
	}

	public function widget( $args, $instance ) {
		$title = isset( $instance['title'] ) ? apply_filters( 'widget_title', $instance['title'] ) : '';
		echo $args['before_widget']; // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- theme markup.
		if ( $title ) {
			echo $args['before_title'] . esc_html( $title ) . $args['after_title']; // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- theme markup.
		}
		echo glot_switcher_html(); // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- static markup.
		echo $args['after_widget']; // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- theme markup.
	}

	public function form( $instance ) {
		$title = isset( $instance['title'] ) ? $instance['title'] : '';
		printf(
			'<p><label for="%1$s">%2$s</label><input class="widefat" id="%1$s" name="%3$s" type="text" value="%4$s"></p>',
			esc_attr( $this->get_field_id( 'title' ) ),
			esc_html__( 'Title:', 'glot' ),
			esc_attr( $this->get_field_name( 'title' ) ),
			esc_attr( $title )
		);
	}

	public function update( $new, $old ) {
		return array( 'title' => sanitize_text_field( isset( $new['title'] ) ? $new['title'] : '' ) );
	}
}
