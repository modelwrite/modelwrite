
const { chromium } = require('@playwright/test');
(async () => {
  const url = 'http://127.0.0.1:8080/ui/projects/coffee-machine/model?branch=main';
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await page.goto(url, { waitUntil: 'networkidle', timeout: 60000 });
  await page.waitForTimeout(800);
  const data = await page.evaluate(() => {
    const rows = Array.from(document.querySelectorAll('table.requirements tbody tr.requirement'));
    return rows.map((tr) => {
      const cells = Array.from(tr.querySelectorAll('td')).map(td => td.textContent.trim());
      const cov = tr.getAttribute('data-mw-coverage');
      // find spans with broken class and their color
      const broken = Array.from(tr.querySelectorAll('.broken')).map(b => ({ text: b.textContent, color: getComputedStyle(b).color, weight: getComputedStyle(b).fontWeight }));
      const relation = Array.from(tr.querySelectorAll('.relation')).map(r => ({ text: r.textContent, color: getComputedStyle(r).color }));
      return { cov, cells, broken, relation };
    });
  });
  // print rows that have 'unresolved' anywhere
  data.forEach((r, i) => {
    if (r.cells.some(c => c.includes('unresolved')) || r.cov === 'uncovered') {
      console.log(JSON.stringify(r, null, 2));
    }
  });
  console.log('total rows:', data.length);
  const covCounts = {};
  data.forEach(r => { covCounts[r.cov] = (covCounts[r.cov]||0)+1; });
  console.log('coverage counts:', JSON.stringify(covCounts));
  await browser.close();
})();
