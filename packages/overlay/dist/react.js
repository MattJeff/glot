"use client";
import{useEffect as n}from"react";function o(e){return n(()=>{process.env.NODE_ENV!=="production"&&(window.GLOT_OVERLAY_MANUAL=!0,import("./index.js").then(r=>r.init(e)))},[]),null}export{o as GlotOverlay};
