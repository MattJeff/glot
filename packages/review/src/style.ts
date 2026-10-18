// Tokens mirror web/app/globals.css (brand oklch 160). Logical properties only (RTL).
export const CSS = `
:host{all:initial}
*{box-sizing:border-box}
.ui{--bg:oklch(1 0 0);--fg:oklch(.145 0 0);--mut:oklch(.45 0 0);--soft:oklch(.97 0 0);--bd:oklch(.88 0 0);--in:oklch(.55 0 0);--acc:oklch(.52 .13 160);--accfg:oklch(.985 0 0);--bad:oklch(.48 .19 27);--warn:oklch(.47 .12 60);--ok:oklch(.45 .12 155);--sh:0 12px 40px -8px rgb(0 0 0/.25),0 2px 6px rgb(0 0 0/.08);
font:14px/1.45 ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif;color:var(--fg);pointer-events:auto}
@media (prefers-color-scheme:dark){.ui{--bg:oklch(.205 0 0);--fg:oklch(.985 0 0);--mut:oklch(.75 0 0);--soft:oklch(.269 0 0);--bd:oklch(.35 0 0);--in:oklch(.6 0 0);--acc:oklch(.72 .16 160);--accfg:oklch(.18 .03 160);--bad:oklch(.72 .19 22);--warn:oklch(.8 .14 75);--ok:oklch(.74 .15 155);--sh:0 12px 40px -8px rgb(0 0 0/.7)}}
[hidden]{display:none!important}
button,select,textarea,input{font:inherit;color:inherit}
button{cursor:pointer}
:focus-visible{outline:2px solid var(--acc);outline-offset:2px}
.sr{position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0 0 0 0);white-space:nowrap}
.bar{position:fixed;display:flex;align-items:center;gap:4px;padding:4px;background:var(--bg);border:1px solid var(--bd);border-radius:12px;box-shadow:var(--sh);inline-size:max-content;max-inline-size:calc(100vw - 16px);flex-wrap:wrap}
.grip{cursor:grab;touch-action:none;inline-size:24px;block-size:36px;border:0;background:none;color:var(--mut);border-radius:6px;display:grid;place-items:center;padding:0}
.ic{display:inline-grid;place-items:center;flex:none}
.btn{display:inline-flex;align-items:center;gap:6px;min-block-size:36px;min-inline-size:36px;justify-content:center;padding:0 10px;border:1px solid transparent;border-radius:8px;background:none;white-space:nowrap}
.btn:hover{background:var(--soft)}
a.btn{text-decoration:none;color:var(--fg)}
.btn[aria-pressed=true],.btn.pri{background:var(--acc);color:var(--accfg)}
.btn.out{border-color:var(--in)}
.btn:disabled{opacity:.5;cursor:not-allowed}
.btn.sm{min-block-size:28px;padding:0 8px;font-size:13px}
.cnt{font-variant-numeric:tabular-nums;font-size:12px;padding:0 6px;border-radius:999px;background:var(--soft);color:var(--fg)}
.loc{font:600 12px ui-monospace,monospace;text-transform:uppercase;padding:2px 6px;border-radius:6px;border:1px solid var(--in)}
input:not([type=checkbox]):not([type=radio]){min-block-size:32px;border:1px solid var(--in);border-radius:8px;background:var(--bg);padding:0 8px;flex:1;min-inline-size:0}
select{min-block-size:36px;border:1px solid var(--in);border-radius:8px;background:var(--bg);padding:0 6px}
.cap{position:fixed;inset:0;cursor:crosshair;touch-action:pan-x pan-y;pointer-events:auto;background:transparent}
.cap:focus{outline:none}
.hl{position:fixed;pointer-events:none;border-radius:6px;outline:2px dashed var(--acc);background:color-mix(in oklch,var(--acc) 10%,transparent);transition:transform .08s ease-out,width .08s,height .08s}
.hl.sel{outline-style:solid}
.tip{position:fixed;pointer-events:none;background:var(--fg);color:var(--bg);font:12px ui-monospace,monospace;padding:3px 8px;border-radius:6px;max-inline-size:min(320px,90vw);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.pin{position:fixed;display:grid;place-items:center;inline-size:28px;block-size:28px;padding:0;border-radius:50% 50% 50% 4px;border:2px solid var(--bg);background:var(--acc);color:var(--accfg);font:700 12px system-ui,sans-serif;box-shadow:var(--sh);animation:pop .18s ease-out}
.pin.done{background:var(--mut);color:var(--bg)}
.pin::after{content:"";position:absolute;inset:-8px}
@keyframes pop{from{opacity:0;transform:scale(.6)}}
.pop,.panel{position:fixed;display:flex;flex-direction:column;gap:10px;padding:12px;background:var(--bg);border:1px solid var(--bd);border-radius:12px;box-shadow:var(--sh);overflow:auto;inline-size:min(380px,calc(100vw - 16px));max-block-size:min(70vh,560px);animation:in .16s ease-out}
@keyframes in{from{opacity:0;transform:translateY(-4px)}}
.head{display:flex;align-items:center;gap:8px}
.pop>.head{position:sticky;top:-12px;margin:-12px -12px 0;padding:8px 12px;background:var(--bg);z-index:1;border-block-end:1px solid var(--bd)}
.pop .foot{position:sticky;bottom:-12px;margin:0 -12px -12px;padding:8px 12px;background:var(--bg);border-block-start:1px solid var(--bd)}
.ttl{flex:1;font-weight:600;margin:0;font-size:14px;min-inline-size:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
code{font:12px ui-monospace,monospace;background:var(--soft);padding:1px 5px;border-radius:4px;overflow-wrap:anywhere}
q,blockquote{display:block;margin:0;padding:6px 10px;border-inline-start:3px solid var(--acc);background:var(--soft);border-radius:4px;quotes:none}
.mut{color:var(--mut);font-size:12px}
.err{color:var(--bad);margin:0}
.okm{color:var(--ok);margin:0}
.wrn{color:var(--warn)}
ol,ul{list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:8px}
.cm{display:flex;flex-direction:column;gap:4px;padding-block-end:8px;border-block-end:1px solid var(--bd)}
.cm p{margin:0;white-space:pre-wrap;overflow-wrap:anywhere}
.sg{border:1px dashed var(--acc);border-radius:6px;padding:4px 8px}
.row{display:flex;gap:6px;align-items:center;flex-wrap:wrap}
.end{justify-content:flex-end}
label{font-weight:600;font-size:13px}
textarea{inline-size:100%;min-block-size:60px;resize:vertical;border:1px solid var(--in);border-radius:8px;background:var(--bg);padding:6px 8px}
.re{display:flex;gap:4px;flex-wrap:wrap}
.re .btn{min-block-size:28px;padding:0 8px;border-color:var(--bd);font-size:13px}
.re .btn[aria-pressed=true]{background:color-mix(in oklch,var(--acc) 18%,var(--bg));color:var(--fg);border-color:var(--acc)}
.badge{font-size:12px;padding:1px 6px;border-radius:4px;border:1px solid currentColor}
.it{all:unset;box-sizing:border-box;display:flex;gap:8px;align-items:flex-start;inline-size:100%;padding:6px;border-radius:8px;cursor:pointer}
.it:hover{background:var(--soft)}
.it:focus-visible{outline:2px solid var(--acc)}
.num{flex:none;display:grid;place-items:center;inline-size:22px;block-size:22px;border-radius:50%;background:var(--acc);color:var(--accfg);font:700 11px system-ui}
.it span:last-child{min-inline-size:0;overflow:hidden}
img.shot{max-inline-size:100%;max-block-size:180px;object-fit:contain;object-position:top;border:1px solid var(--bd);border-radius:6px}
@media (max-width:520px){.lbl{display:none}.bar{flex-wrap:nowrap}.btn{padding:0 8px}}
fieldset.props{border:1px solid var(--bd);border-radius:8px;padding:6px 8px;margin:0;display:flex;flex-direction:column;gap:6px}
legend{font-weight:600;font-size:13px;padding:0 4px}
.prop{display:flex;gap:8px;align-items:flex-start;font-weight:400;padding:4px;border-radius:6px;cursor:pointer}
.prop:has(:checked){background:var(--soft)}
.prop input{margin-block-start:4px}
input[type=checkbox],input[type=radio]{accent-color:var(--acc);inline-size:16px;block-size:16px}
mark{background:color-mix(in oklch,var(--acc) 25%,transparent);color:inherit;border-radius:3px;padding:0 2px}
@media (prefers-reduced-motion:reduce){*{animation:none!important;transition:none!important}}
`;
