// No build step: plain script on the editor's globals.
( function ( blocks, el, i18n, be ) {
	blocks.registerBlockType( 'glot/switcher', {
		edit: function () {
			return el.createElement(
				'div',
				be.useBlockProps(),
				el.createElement( 'select', { disabled: true, 'aria-label': i18n.__( 'Language', 'glot' ) },
					el.createElement( 'option', null, i18n.__( 'Language switcher (shown on the site)', 'glot' ) ) )
			);
		},
		save: function () {
			return null;
		},
	} );
} )( window.wp.blocks, window.wp.element, window.wp.i18n, window.wp.blockEditor );
