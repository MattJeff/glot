<?php
/**
 * Front end of the glot/switcher block.
 *
 * @package glot
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}
?>
<div <?php echo get_block_wrapper_attributes(); // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- core-escaped. ?>><?php echo glot_switcher_html(); // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- static markup. ?></div>
