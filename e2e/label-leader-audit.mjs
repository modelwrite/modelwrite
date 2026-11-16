
import fs from 'node:fs';
for (const file of process.argv.slice(2)) {
  const svg = fs.readFileSync(file, 'utf8');
  const rects = [...svg.matchAll(/<rect class='node-rect'[^>]*x='([\d.-]+)' y='([\d.-]+)' width='([\d.-]+)' height='([\d.-]+)'/g)]
    .map(m => ({x:+m[1], y:+m[2], w:+m[3], h:+m[4]}));
  const leaders = [...svg.matchAll(/<polyline class='edge-leader' points='([^']*)'/g)].map(m => m[1].trim().split(/\s+/).map(p => p.split(',').map(Number)));
  let crossing = 0;
  for (const pts of leaders) {
    let hit = false;
    for (let i = 0; i+1 < pts.length && !hit; i++) {
      const [a, b] = [pts[i], pts[i+1]];
      for (const r of rects) {
        // Liang-Barsky against the box's interior
        const d = [b[0]-a[0], b[1]-a[1]];
        const p = [-d[0], d[0], -d[1], d[1]];
        const q = [a[0]-r.x, r.x+r.w-a[0], a[1]-r.y, r.y+r.h-a[1]];
        let t0 = 0, t1 = 1, ok = true;
        for (let k = 0; k < 4; k++) {
          if (Math.abs(p[k]) < 1e-12) { if (q[k] <= 0) { ok = false; break; } }
          else { const t = q[k]/p[k]; if (p[k] < 0) { if (t > t1) { ok = false; break; } t0 = Math.max(t0, t); } else { if (t < t0) { ok = false; break; } t1 = Math.min(t1, t); } }
        }
        if (ok && t0 < t1) { hit = true; break; }
      }
    }
    if (hit) crossing += 1;
  }
  console.log(file.padEnd(40), 'leaders', String(leaders.length).padEnd(4), 'leaders passing behind a box', crossing);
}
