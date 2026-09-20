
const { chromium } = require('@playwright/test');
const fs = require('fs');
(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const pngPath = process.cwd() + '/ui-lower.png';
  const b64 = fs.readFileSync(pngPath).toString('base64');
  await page.setContent('<html><body style="margin:0"><img id="i" src="data:image/png;base64,' + b64 + '"></body></html>');
  await page.waitForTimeout(300);
  const result = await page.evaluate(() => {
    const img = document.getElementById('i');
    const c = document.createElement('canvas');
    c.width = img.naturalWidth; c.height = img.naturalHeight;
    const ctx = c.getContext('2d');
    ctx.drawImage(img, 0, 0);
    const W = img.naturalWidth, H = img.naturalHeight;
    // sample columns: right panel region approx x from W-320 to W-8
    function columnStats(x0, x1, label) {
      let nonWhite = 0, total = 0;
      const bands = [];
      for (let y = 0; y < H; y += 10) {
        let rowNonWhite = 0, rowTotal = 0;
        for (let x = x0; x < x1; x += 4) {
          const d = ctx.getImageData(x, y, 1, 1).data;
          const white = d[0] > 245 && d[1] > 245 && d[2] > 245;
          if (!white) rowNonWhite++;
          rowTotal++;
        }
        nonWhite += rowNonWhite; total += rowTotal;
        bands.push({ y, nonWhite: rowNonWhite });
      }
      return { label, x0, x1, nonWhite, total, firstNonWhiteBand: bands.find(b=>b.nonWhite>2)?.y ?? null, lastNonWhiteBand: bands.slice().reverse().find(b=>b.nonWhite>2)?.y ?? null };
    }
    return {
      W, H,
      right: columnStats(W - 320, W - 8, 'right-col'),
      left: columnStats(8, 280, 'left-col'),
      center: columnStats(W/2 - 100, W/2 + 100, 'center'),
    };
  });
  console.log(JSON.stringify(result, null, 2));
  await browser.close();
})();
