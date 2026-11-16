
import fs from 'node:fs';
const { chromium } = await import('playwright');
const raw = fs.readFileSync(process.argv[2],'utf8').replace(/^<\?xml[^>]*\?>\s*/,'');
const want = process.argv[3];
const b = await chromium.launch();
const p = await b.newPage({viewport:{width:1920,height:1080}});
await p.setContent('<!doctype html><html><body style="margin:0;background:#fff"></body></html>');
await p.evaluate((h)=>{document.body.innerHTML=h;}, raw);
const out = await p.evaluate((want)=>{
  const groups=[...document.querySelectorAll('g.edge')];
  const res=[];
  groups.forEach((g,gi)=>{
    const t=g.querySelector('text.edge-label'); if(!t||t.textContent!==want) return;
    const bb=t.getBBox();
    res.push({label:t.textContent, box:[bb.x,bb.y,bb.width,bb.height].map(v=>+v.toFixed(1))});
    groups.forEach((o,oi)=>{
      const el=o.querySelector('polyline.mw-edge, path.mw-edge, line.mw-edge'); if(!el) return;
      const total=el.getTotalLength(); const n=Math.max(200,Math.ceil(total/1.5));
      const hits=[];
      for(let i=0;i<=n;i++){ const q=el.getPointAtLength(total*i/n);
        if(q.x>bb.x&&q.x<bb.x+bb.width&&q.y>bb.y&&q.y<bb.y+bb.height) hits.push([+q.x.toFixed(1),+q.y.toFixed(1)]); }
      if(hits.length) res.push({other:oi, own:oi===gi, kind:o.getAttribute('data-mw-kind'), hasJump:/Q /.test(el.getAttribute('d')||''), d:(el.getAttribute('d')||el.getAttribute('points')||'').slice(0,140), hits:hits.length, first:hits[0]});
    });
  });
  return res;
}, want);
await b.close();
console.log(JSON.stringify(out,null,1));
