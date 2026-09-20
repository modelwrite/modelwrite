
const { chromium } = require('@playwright/test');
(async () => {
  const url = 'http://127.0.0.1:8080/ui/projects/coffee-machine/model?branch=main';
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await page.goto(url, { waitUntil: 'networkidle', timeout: 60000 });
  await page.waitForTimeout(800);
  const inspect = async (label) => {
    const data = await page.evaluate(() => {
      const pick = (sel) => {
        const el = document.querySelector(sel);
        if (!el) return null;
        const r = el.getBoundingClientRect();
        const cs = getComputedStyle(el);
        return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, width: r.width, height: r.height,
          position: cs.position, alignSelf: cs.alignSelf, overflow: cs.overflow, maxHeight: cs.maxHeight };
      };
      return {
        scrollY: window.scrollY,
        docHeight: document.documentElement.scrollHeight,
        workbench: pick('.mw-workbench'),
        tree: pick('.mw-tree-panel'),
        props: pick('.mw-props-panel'),
        content: pick('.mw-content'),
        main: pick('main'),
      };
    });
    console.log('--- ' + label + ' ---');
    console.log(JSON.stringify(data, null, 2));
  };
  await inspect('TOP');
  // scroll to requirements
  await page.evaluate(() => {
    const heads = Array.from(document.querySelectorAll('h2, h3'));
    const t = heads.find((x) => /requirement/i.test(x.textContent));
    if (t) t.scrollIntoView();
    else window.scrollBy(0, 1200);
  });
  await page.waitForTimeout(600);
  await inspect('REQUIREMENTS');
  await browser.close();
})();
