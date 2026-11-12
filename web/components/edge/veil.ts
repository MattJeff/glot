// Inline first part of the website tag (no import: packages/edge/test/website.test.ts loads this file as is).
// An async script may run after the browser's first paint; this runs before it, without any request: when
// localStorage says this visitor reads one of the languages the tag offers (`glot-w:langs`, written by glot.js
// while the tag is on; `glot-locale` = their saved choice), the page is hidden until glot.js has applied the
// cached translations — 1 s at most, then it shows whatever happened (glot.js removes `#glot-veil` sooner).
export const VEIL =
  '<script>try{var g=localStorage,l=(g.getItem("glot-locale")||navigator.language).split("-")[0].toLowerCase();if((" "+(g.getItem("glot-w:langs")||"")+" ").indexOf(" "+l+" ")>=0){var s=document.createElement("style");s.id="glot-veil";s.textContent="html{opacity:0!important}";document.head.appendChild(s);setTimeout(function(){s.remove()},1000)}}catch(e){}</script>';
