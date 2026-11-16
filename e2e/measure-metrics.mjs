
import fs from 'node:fs';
const raw = fs.readFileSync('e2e/svg-scope-full.svg','utf8').replace(/^<\?xml[^>]*\?>\s*/,'');
const { chromium } = await import('playwright');
const b = await chromium.launch();
const p = await b.newPage({viewport:{width:1920,height:1080}});
await p.setContent('<!doctype html><html><body style="margin:0;background:#fff"></body></html>');
await p.evaluate((h)=>{document.body.innerHTML=h;}, raw);
const rows = await p.evaluate(()=>{
  const svg=document.querySelector('svg');
  svg.setAttribute('style','width:1920px;height:1080px');
  const out=[];
  for (const t of document.querySelectorAll('text.edge-label')) {
    const bb=t.getBBox();
    out.push({text:t.textContent, chars:t.textContent.length, w:bb.width, h:bb.height, fs:parseFloat(getComputedStyle(t).fontSize)});
  }
  return out;
});
await b.close();
const per = rows.map(r=>({chars:r.chars, w:+r.w.toFixed(2), perChar:+(r.w/r.chars).toFixed(3), h:+r.h.toFixed(2), fs:r.fs}));
per.sort((a,b)=>b.perChar-a.perChar);
console.log('fontSizes', JSON.stringify([...new Set(rows.map(r=>r.fs))]));
console.log('height (all):', JSON.stringify([...new Set(per.map(r=>r.h))]));
console.log('widest per-char:'); console.log(JSON.stringify(per.slice(0,12),null,0));
console.log('narrowest per-char:'); console.log(JSON.stringify(per.slice(-6),null,0));
const tiny = per.filter(r=>r.chars<=4); console.log('short labels:', JSON.stringify(tiny,null,0));
