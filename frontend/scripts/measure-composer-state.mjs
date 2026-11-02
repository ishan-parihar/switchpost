// Drive the composer OPEN and measure the states the M-1 phase claims are
// broken. The route-level probe renders /posts/new's DEFAULT state only, and
// the composer is a modal behind a click (AGENTS.md 9.35, 9.50) - so these
// numbers exist nowhere in the source and nowhere in a route-level pass.
//
// Measured here:
//   1. the overlap the probe reports (the signature text vs Preview / Save
//      Draft / Post Now) - WHICH elements, and how much they overlap.
//   2. the scrim: its computed background, and what it costs in contrast.
//   3. the preview pane's rendered width vs the editor's, at each width -
//      the plan's claim is that the preview is unreachable below `lg`.
//   4. the footer's primary buttons: which of them are FILLED (the plan says
//      two are, so neither reads as the primary).
import { chromium } from 'playwright-core';

const EXE = process.env.PW_EXE || '/home/ishanp/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome';
const BASE = process.env.SP_URL || 'http://127.0.0.1:6543';
const CV = process.env.CV;

if (!CV) { console.log('SKIP: no CV'); process.exit(0); }

const WIDTHS = [1440, 768, 390];

const browser = await chromium.launch({ executablePath: EXE, args: ['--no-sandbox'] });

function measure() {
  const dialog = [...document.querySelectorAll('[role="dialog"], [role="alertdialog"]')].pop();
  if (!dialog) return { error: 'no composer dialog' };

  // The two columns, when the preview is in its DEFAULT state.
  const editor = dialog.querySelector('[class*="flex-col"]');
  const pane = dialog.querySelector('[data-preview-pane]')
    || [...dialog.querySelectorAll('div')].find((d) => /preview/i.test(d.className || ''));
  const rect = (el) => {
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { w: Math.round(r.width), h: Math.round(r.height), top: Math.round(r.top), left: Math.round(r.left) };
  };

  // Overlap pairs inside the dialog, using the probe's own rule.
  const boxes = [...dialog.querySelectorAll('*')].filter((el) => {
    if (el.closest('.truncate')) return false;
    const cs = getComputedStyle(el);
    if (cs.position === 'absolute' || cs.position === 'fixed' || cs.position === 'relative') return false;
    if (cs.transform !== 'none' || cs.visibility === 'hidden') return false;
    return Array.from(el.childNodes).some((n) => n.nodeType === Node.TEXT_NODE && n.textContent.trim().length > 0);
  }).map((el) => ({ el, r: el.getBoundingClientRect() }))
    .filter((b) => b.r.width > 0 && b.r.height > 0);
  const overlaps = [];
  for (let i = 0; i < boxes.length; i++) {
    for (let j = i + 1; j < boxes.length; j++) {
      const ea = boxes[i].el;
      const eb = boxes[j].el;
      if (ea.contains(eb) || eb.contains(ea)) continue;
      const a = boxes[i].r;
      const b = boxes[j].r;
      const ox = Math.min(a.right, b.right) - Math.max(a.left, b.left);
      const oy = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
      if (ox > 2 && oy > 2) {
        const area = Math.min(a.width * a.height, b.width * b.height);
        if (ox * oy > area * 0.35) {
          overlaps.push({
            a: ea.textContent.trim().slice(0, 30),
            b: eb.textContent.trim().slice(0, 30),
            aPath: (ea.className || '').toString().slice(0, 50),
            bPath: (eb.className || '').toString().slice(0, 50),
            ox: Math.round(ox), oy: Math.round(oy),
          });
        }
      }
    }
  }

  // The scrim: the overlay div behind the panel.
  const scrim = dialog.parentElement?.querySelector(':scope > div[aria-hidden="true"]')
    || [...dialog.querySelectorAll('div')].find((d) => /bg-overlay/.test(d.className || ''));

  // Footer buttons: which are FILLED (a background, not just a border).
  const buttons = [...dialog.querySelectorAll('button, a[role="button"]')].map((b) => {
    const cs = getComputedStyle(b);
    return {
      text: (b.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 22),
      filled: cs.backgroundColor !== 'rgba(0, 0, 0, 0)' && cs.backgroundColor !== 'transparent',
      bg: cs.backgroundColor,
      color: cs.color,
    };
  }).filter((b) => b.text);

  // The signature / editor area: where does the signature text sit?
  const sig = [...dialog.querySelectorAll('*')].find((el) =>
    /^[—–-]\s/.test((el.textContent || '').trim()) && el.children.length === 0);

  return {
    dialogBox: rect(dialog),
    editor: rect(editor),
    pane: rect(pane),
    paneVisible: pane ? getComputedStyle(pane).display !== 'none' : false,
    paneClass: (pane?.className || '').toString().slice(0, 120),
    scrim: scrim ? { cls: (scrim.className || '').toString().slice(0, 60), bg: getComputedStyle(scrim).backgroundColor } : null,
    overlaps,
    footerButtons: buttons.filter((b) => /Post Now|Schedule|Save Draft|Draft|Cancel|Preview/i.test(b.text)),
    signature: sig ? { text: (sig.textContent || '').slice(0, 40), box: rect(sig), cls: (sig.className || '').toString().slice(0, 60) } : null,
    previewToggle: (() => {
      const t = [...dialog.querySelectorAll('button')].find((b) => /^preview$|^hide preview$/i.test((b.textContent || '').trim()));
      return t ? { text: t.textContent.trim(), cls: (t.className || '').toString().slice(0, 90) } : null;
    })(),
  };
}

try {
  for (const width of WIDTHS) {
    const ctx = await browser.newContext({
      viewport: { width, height: 900 }, colorScheme: 'dark', ignoreHTTPSErrors: true,
    });
    await ctx.addCookies([{ name: 'sf_session', value: CV, domain: new URL(BASE).hostname, path: '/', httpOnly: false }]);
    const page = await ctx.newPage();
    await page.goto(BASE + '/login', { waitUntil: 'domcontentloaded' });
    await page.evaluate(() => { try { localStorage.setItem('switchpost-onboarded', '1'); } catch (e) {} });
    await page.goto(BASE + '/posts/new', { waitUntil: 'domcontentloaded', timeout: 40000 });
    await page.waitForTimeout(5000);
    for (let i = 0; i < 4; i++) { await page.keyboard.press('Escape'); await page.waitForTimeout(250); }

    // OPEN the composer. The route renders the composer as its default state
    // on /posts/new, so it is usually already there.
    let opened = await page.evaluate(() => !!document.querySelector('[role="dialog"]'));
    if (!opened) {
      const btn = await page.evaluate(() => {
        const b = [...document.querySelectorAll('button')].find((x) => /new post|compose/i.test(x.textContent || ''));
        if (b) { b.click(); return true; }
        return false;
      });
      if (btn) { await page.waitForTimeout(2500); opened = await page.evaluate(() => !!document.querySelector('[role="dialog"]')); }
    }

    const m = await page.evaluate(measure);
    console.log(`\n===== ${width}px (composer open: ${opened}) =====`);
    if (m.error) { console.log(m.error); await ctx.close(); continue; }
    console.log('dialog:', JSON.stringify(m.dialogBox));
    console.log('editor:', JSON.stringify(m.editor), ' pane:', JSON.stringify(m.pane), ' paneVisible:', m.paneVisible);
    console.log('paneClass:', m.paneClass);
    console.log('scrim:', JSON.stringify(m.scrim));
    console.log('previewToggle:', JSON.stringify(m.previewToggle));
    console.log('signature:', JSON.stringify(m.signature));
    console.log('footerButtons:', JSON.stringify(m.footerButtons, null, 1));
    console.log(`OVERLAPS (${m.overlaps.length}):`);
    for (const o of m.overlaps.slice(0, 8)) {
      console.log(`   "${o.a}" x "${o.b}"  ${o.ox}x${o.oy}px`);
      console.log(`      a: ${o.aPath}`);
      console.log(`      b: ${o.bPath}`);
    }
    await ctx.close();
  }
} finally {
  await browser.close();
}
