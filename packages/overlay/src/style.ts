export const CSS = `
:host{all:initial}
*{box-sizing:border-box}
.pop,.pill{--bg:#ffffff;--fg:#18181b;--mut:#71717a;--bd:#e4e4e7;--in:#fafafa;--acc:#4f46e5;--accfg:#fff;--ok:#15803d;--okbg:#dcfce7;--warn:#a16207;--warnbg:#fef9c3;--bad:#b91c1c;--badbg:#fee2e2;--sh:0 12px 40px -8px rgba(0,0,0,.25),0 2px 6px rgba(0,0,0,.08)}
@media (prefers-color-scheme:dark){.pop,.pill{--bg:#18181b;--fg:#f4f4f5;--mut:#a1a1aa;--bd:#2e2e33;--in:#111113;--acc:#818cf8;--accfg:#111;--ok:#4ade80;--okbg:#14301e;--warn:#facc15;--warnbg:#3a3010;--bad:#f87171;--badbg:#3b1414;--sh:0 12px 40px -8px rgba(0,0,0,.7)}}
.pop{position:fixed;display:none;flex-direction:column;gap:8px;padding:12px;overflow:auto;pointer-events:auto;background:var(--bg);color:var(--fg);border:1px solid var(--bd);border-radius:12px;box-shadow:var(--sh);font:13px/1.45 ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif;animation:in .12s ease-out}
.pop.open{display:flex}
@keyframes in{from{opacity:0;transform:translateY(-4px)}}
.hl{position:fixed;display:none;border-radius:6px;pointer-events:none;transition:all .06s}
.hl.hover{outline:2px dashed #6366f1;background:rgba(99,102,241,.08)}
.hl.sel{outline:2px solid #6366f1;background:rgba(99,102,241,.12)}
.pill{position:fixed;display:none;left:50%;bottom:16px;transform:translateX(-50%);padding:6px 12px;border-radius:999px;background:var(--fg);color:var(--bg);font:12px system-ui,sans-serif;box-shadow:var(--sh)}
.pill.open{display:block}
.head{display:flex;align-items:center;gap:8px}
.title{flex:1;font-weight:600;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.mono,code,.code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px}
code{background:var(--in);border:1px solid var(--bd);padding:0 4px;border-radius:4px}
.muted{color:var(--mut)}.err{color:var(--bad);margin:0}.okmsg{color:var(--ok);margin:0}
p{margin:0}
blockquote{margin:0;padding:6px 10px;border-left:3px solid var(--acc);background:var(--in);border-radius:4px}
.meta{display:flex;flex-wrap:wrap;gap:6px 10px;align-items:center;color:var(--mut);font-size:12px}
.note{font-size:12px;color:var(--mut);font-style:italic}
.chips{display:flex;flex-wrap:wrap;gap:4px}
.chip{all:unset;cursor:pointer;font:11px ui-monospace,monospace;padding:2px 8px;border-radius:999px;border:1px solid var(--bd);color:var(--mut)}
.chip.on{border-color:var(--acc);color:var(--acc)}
.locs{display:flex;flex-direction:column;gap:6px}
.loc{display:flex;flex-direction:column;gap:3px;padding:2px 0 2px 8px;border-left:2px solid var(--bd)}
.loc.shown{border-left-color:var(--acc)}
.lochead{display:flex;align-items:center;gap:6px;font-size:11px}
.code{font-weight:600;text-transform:uppercase}
.tag{color:var(--mut);border:1px solid var(--bd);border-radius:4px;padding:0 4px}
.status{margin-left:auto;color:var(--mut)}.status.ok{color:var(--ok)}.status.bad{color:var(--bad)}
.in{width:100%;resize:none;overflow:hidden;font:inherit;color:var(--fg);background:var(--in);border:1px solid var(--bd);border-radius:6px;padding:5px 8px;outline:none}
.in:focus{border-color:var(--acc);box-shadow:0 0 0 3px color-mix(in srgb,var(--acc) 20%,transparent)}
textarea.in::placeholder{color:var(--bad);opacity:.7}
.issues{display:flex;flex-wrap:wrap;gap:4px}
.badge{display:inline-block;font-size:11px;padding:1px 6px;border-radius:4px;background:var(--in);color:var(--mut)}
.badge.ok{background:var(--okbg);color:var(--ok)}.badge.warn{background:var(--warnbg);color:var(--warn)}.badge.bad{background:var(--badbg);color:var(--bad)}
.row{display:flex;gap:6px;align-items:center}
.ai{display:flex;flex-wrap:wrap;gap:6px;align-items:center}
.locs{padding-top:8px;border-top:1px solid var(--bd)}
.btn{all:unset;cursor:pointer;font:12px system-ui,sans-serif;padding:4px 10px;border-radius:6px;border:1px solid var(--bd);color:var(--fg);white-space:nowrap}
.btn:hover{background:var(--in)}
.btn.primary{background:var(--acc);border-color:var(--acc);color:var(--accfg)}
.btn.primary:hover{filter:brightness(1.1)}
.icon{all:unset;cursor:pointer;width:22px;height:22px;text-align:center;border-radius:6px;color:var(--mut);font-size:16px;line-height:22px}
.icon:hover{background:var(--in);color:var(--fg)}
.link{all:unset;cursor:pointer;color:var(--acc);font-size:11px}
.sugg{display:flex;flex-direction:column;gap:4px}
.suggestion{all:unset;cursor:pointer;padding:6px 8px;border-radius:6px;border:1px dashed var(--bd)}
.suggestion:hover{border-color:var(--acc);background:var(--in)}
.btn:focus-visible,.chip:focus-visible,.icon:focus-visible,.link:focus-visible,.suggestion:focus-visible{outline:2px solid var(--acc);outline-offset:1px}
`;
