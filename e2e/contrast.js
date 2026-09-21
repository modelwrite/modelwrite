// SPDX-License-Identifier: AGPL-3.0-or-later
// Measure the RENDERED contrast of representative text on a page, against the first
// opaque background above it. WCAG 2.x relative luminance; exits 1 on any failure.
// Dev-only; never part of the product.
// Usage: node contrast.js <file-or-url> [more...]
const { chromium } = require('@playwright/test');
const hexToRgb = (s) => { const m = s.match(/rgba?\(([^)]+)\)/); if (!m) return null; const p = m[1].split(',').map((x) => parseFloat(x)); return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 }; };
const lum = (c) => { const f = (v) => { const s = v / 255; return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4); }; return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b); };
const ratio = (a, b) => { const la = lum(a), lb = lum(b); const hi = Math.max(la, lb), lo = Math.min(la, lb); return (hi + 0.05) / (lo + 0.05); };
const blend = (fg, bg) => ({ r: fg.r * fg.a + bg.r * (1 - fg.a), g: fg.g * fg.a + bg.g * (1 - fg.a), b: fg.b * fg.a + bg.b * (1 - fg.a), a: 1 });
(async () => {
  const targets = process.argv.slice(2);
  const browser = await chromium.launch();
  let worst = 99;
  let fails = 0;
  for (const target of targets) {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    await page.goto(target, { waitUntil: 'networkidle', timeout: 60000 });
    await page.waitForTimeout(400);
    const rows = await page.evaluate(() => {
      const picks = [
        ['body copy', 'p'],
        ['nav link', '.masthead nav a'],
        ['eyebrow/label', '.eyebrow'],
        ['chip label', '.chips li'],
        ['primary button', '.button'],
        ['quiet button', '.button--quiet'],
        ['figcaption', 'figcaption'],
        ['limits body', '.limits p'],
        ['gaps body', '.gaps p'],
        ['door body', '.door p'],
        ['door host (mono)', '.door__host'],
        ['statusline', '.statusline'],
        ['repo link (chrome)', '.masthead .repo'],
        ['wordmark (chrome)', '.wordmark'],
        ['faq body', '.faq p'],
        ['table cell', '.examples td'],
        ['footer text', '.site-footer p'],
      ];
      const out = [];
      const seen = new Set();
      for (const [label, sel] of picks) {
        const el = document.querySelector(sel);
        if (!el) { out.push({ label, sel, missing: true }); continue; }
        if (seen.has(sel)) continue;
        seen.add(sel);
        const cs = getComputedStyle(el);
        // effective background: walk up to the first opaque ancestor
        let node = el, bg = null;
        while (node && node !== document.documentElement) {
          const b = getComputedStyle(node).backgroundColor;
          const m = b.match(/rgba?\(([^)]+)\)/);
          if (m) {
            const p = m[1].split(',').map(parseFloat);
            const a = p.length > 3 ? p[3] : 1;
            if (a > 0.999) { bg = b; break; }
            if (a > 0 && !bg) bg = b; // remember a translucent one, keep walking
          }
          node = node.parentElement;
        }
        out.push({ label, sel, color: cs.color, bg: bg || 'rgb(255,255,255)', size: cs.fontSize, weight: cs.fontWeight, text: (el.textContent || '').trim().slice(0, 34) });
      }
      return out;
    });
    console.log('=== ' + target);
    for (const r of rows) {
      if (r.missing) { console.log('  ' + r.label.padEnd(22) + ' (not on this page)'); continue; }
      const fg = hexToRgb(r.color), bg0 = hexToRgb(r.bg);
      if (!fg || !bg0) { console.log('  ' + r.label.padEnd(22) + ' unparsed ' + r.color + ' / ' + r.bg); continue; }
      const bg = bg0.a < 1 ? blend(bg0, { r: 255, g: 255, b: 255, a: 1 }) : bg0;
      const fg2 = fg.a < 1 ? blend(fg, bg) : fg;
      const cr = ratio(fg2, bg);
      const px = parseFloat(r.size);
      const large = px >= 24 || (px >= 18.66 && parseInt(r.weight, 10) >= 700);
      const need = large ? 3 : 4.5;
      const ok = cr >= need;
      if (!ok) fails++;
      if (cr < worst) worst = cr;
      console.log('  ' + (ok ? 'PASS' : 'FAIL') + '  ' + r.label.padEnd(22) + cr.toFixed(2) + ':1  (' + r.size + ' w' + r.weight + ', needs ' + need + ')  ' + r.color + ' on ' + r.bg);
    }
    await page.close();
  }
  await browser.close();
  console.log('worst ratio: ' + worst.toFixed(2) + ':1   failures: ' + fails);
  process.exitCode = fails === 0 ? 0 : 1;
})();