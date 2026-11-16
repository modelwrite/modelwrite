
import fs from 'node:fs';
const raw = fs.readFileSync('e2e/label-before.svg','utf8').replace(/^<\?xml[^>]*\?>\s*/,'');
const { chromium } = await import('playwright');
const b = await chromium.launch();
const p = await b.newPage({viewport:{width:1920,height:1080}});
await p.setContent('<!doctype html><html><body style="margin:0;background:#fff"></body></html>');
await p.evaluate((h)=>{document.body.innerHTML=h;}, raw);
const rows = await p.evaluate(()=>{
  const out=[];
  for (const t of document.querySelectorAll('text.edge-label')) {
    const bb=t.getBBox();
    out.push({text:t.textContent, yAttr:parseFloat(t.getAttribute('y')), bbY:bb.y, bbH:bb.height, bbW:bb.width});
  }
  return out;
});
await b.close();
const ascent = rows.map(r=>+(r.yAttr-r.bbY).toFixed(3));
const descent = rows.map(r=>+(r.bbY+r.bbH-r.yAttr).toFixed(3));
console.log('ascent', JSON.stringify([...new Set(ascent)]));
console.log('descent', JSON.stringify([...new Set(descent)]));
console.log('height', JSON.stringify([...new Set(rows.map(r=>+r.bbH.toFixed(3)))]));
