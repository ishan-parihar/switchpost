// Deep state walker: drives states NO existing gate reaches, measures the
// same four hazards at 1440/375, exits 1 on real findings.
import { chromium } from 'playwright-core';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const CHROME_CANDIDATES = [
  process.env.CHROME_PATH,
  ...fs.readdirSync(path.join(os.homedir(), '.cache/ms-playwright'), { withFileTypes: true })
    .filter((d) => d.isDirectory() && d.name.startsWith('chromium-'))
    .flatMap((d) => [
      path.join(os.homedir(), '.cache/ms-playwright', d.name, 'chrome-linux64/chrome'),
      path.join(os.homedir(), '.cache/ms-playwright', d.name, 'chrome-linux/chrome'),
    ]),
].filter(Boolean);
const CHROME = CHROME_CANDIDATES.find((p) => fs.existsSync(p));
if (!CHROME) { console.error('BLOCKED: no chromium'); process.exit(3); }

import { CONTRAST_CORE } from './contrast-core.mjs';

const arg = (n, d) => { const i = process.argv.indexOf(`--${n}`); return i > -1 && process.argv[i+1] ? process.argv[i+1] : d; };
const BASE = arg('url', 'https://localhost:6544');
const PASSWORD = arg('password', '');

const findings = [];
async function audit(page, where) {
  // The body runs in the page; the contrast measurement is the ONE shared
  // implementation (contrast-core.mjs), spliced textually because the
  // function is serialized into the browser where module imports don't exist.
  const data = await page.evaluate(new Function(CONTRAST_CORE + `
return (() => {
    const out = [];
    const isHidden = (el) => {
      if (!el.isConnected) return true;
      const s = getComputedStyle(el);
      if (s.display==='none'||s.visibility==='hidden'||+s.opacity===0) return true;
      const r = el.getBoundingClientRect();
      return r.width===0 && r.height===0;
    };
    const docOver = document.documentElement.scrollWidth - document.documentElement.clientWidth;
    if (docOver > 1) out.push({kind:'doc-overflow', desc:\`\${docOver}px\`});
    for (const el of document.querySelectorAll('button, a[href], input, select, textarea, [role=button], [role=tab], [role=checkbox], [role=switch], summary, label:has(input)')) {
      if (isHidden(el) || el.closest('[aria-hidden="true"]')) continue;
      if (el.disabled || el.getAttribute('aria-disabled')==='true') continue;
      const r = el.getBoundingClientRect();
      if (r.width>0 && r.height>0 && (r.width<24 || r.height<24)) {
        // a 44px (or 24px+) clickable wrapper is the real target (checkbox labels,
        // hover-reveal overlays): same exemption the repo's audit-targets makes.
        // opacity-0-until-hover wrappers are the reveal pattern, not hidden controls.
        const lab = el.closest('label');
        if (lab) { const lr = lab.getBoundingClientRect(); if (lr.width>=24 && lr.height>=24) continue; }
        const wrap = el.parentElement;
        if (wrap && /cursor-pointer|cursor-grab/.test(getComputedStyle(wrap).cursor||'')) { const wr = wrap.getBoundingClientRect(); if (wr.width>=24 && wr.height>=24) continue; }
        if ((el.textContent||'').trim().startsWith('Skip to main content')) continue;
        out.push({kind:'tiny-target', desc:\`\${Math.round(r.width)}x\${Math.round(r.height)} \${el.textContent.trim().slice(0,30)||el.type||el.tagName}\`, tag: el.tagName});
      }
    }
    // (the shared parseColor/lum/effectiveBg arrive via the CONTRAST_CORE prefix)
    // skip-link is the standard visually-hidden-until-focus a11y pattern
    const isSkip = (el) => (el.textContent||'').trim().startsWith('Skip to main content');
    for (const el of document.querySelectorAll('body *')) {
      if (isHidden(el) || el.closest('[aria-hidden="true"]') || isSkip(el)) continue;
      if (el.classList.contains('truncate')) continue;
      const own = [...el.childNodes].some((n)=>n.nodeType===3 && n.textContent.trim());
      if (!own) continue;
      const cs = getComputedStyle(el);
      const fg = parseColor(cs.color); if (!fg) continue;
      const bg = effectiveBg(el);
      const r1 = lum(fg), r2 = lum(bg);
      const ratio = (Math.max(r1,r2)+0.05)/(Math.min(r1,r2)+0.05);
      const size = parseFloat(cs.fontSize);
      const bold = (parseInt(cs.fontWeight)||400) >= 700;
      const floor = (size>=24 || (size>=18.66 && bold)) ? 3 : 4.5;
      if (ratio < floor - 0.05) out.push({kind:'low-contrast', desc:\`\${ratio.toFixed(2)}:1 "\${el.textContent.trim().slice(0,36)}"\`, fs:size});
    }
    return out;
})();`));

  for (const d of data) findings.push({where, ...d});
}

const findBtn = (page, text, scope='document') => page.evaluate((t) => {
  const els = [...document.querySelectorAll('button, a, [role=button], [role=tab], summary')];
  const el = els.find((e) => !e.closest('[aria-hidden=true]') && e.offsetParent && (e.textContent||'').trim().includes(t));
  if (el) { el.click(); return true; } return false;
}, text);

async function main() {
  // A missing password is a missing PRECONDITION, not a crash: every
  // gate here exits 3 (BLOCKED) when it cannot run (AGENTS.md §12),
  // and a fill() timeout 30s deep in the run is that failure reported
  // in the least useful way.
  if (!PASSWORD) { console.error('BLOCKED: no --password'); process.exit(3); }
  const browser = await chromium.launch({ executablePath: CHROME, args: ['--ignore-certificate-errors'] });
  const page = await browser.newPage();
  await page.setViewportSize({width:1440, height:900});
  await page.goto(`${BASE}/login`, {waitUntil:'load'});
  // The password field is client-rendered (SvelteKit SPA shell): wait
  // for it, and report BLOCKED if the app never comes up rather than
  // timing out inside fill().
  try {
    await page.waitForSelector('input[type=password]', { timeout: 20000 });
  } catch {
    console.error('BLOCKED: login page never rendered a password field');
    process.exit(3);
  }
  await page.fill('input[type=password]', PASSWORD);
  await page.evaluate(() => document.querySelector('input[type=password]').closest('form').requestSubmit());
  await page.waitForTimeout(2500);

  // ── THE ONBOARDING TOUR, and this walker has never dismissed it ──
  //
  // MEASURED 2026-10-31: with this script's own login and no dismissal,
  // the first-run tour is OPEN on every route it walks - /media,
  // /comments, /dms, /automation, /kanban, /search - with
  // `localStorage['switchpost-onboarded']` null. It is a full-screen
  // role="dialog", so it is part of the population this script measures.
  // Counting exactly, across those six routes:
  //
  //   tour open   population 541, of which 102 controls live IN the tour
  //   tour closed  population 439
  //
  // So 19% of what this gate measures is an overlay the founder sees once.
  // A walker whose job is "states no existing gate reaches" was measuring
  // one state every gate already reaches, and reporting 0 findings from it.
  //
  // This script logs in by SUBMITTING the form on /login, so the page is
  // already on the origin and the storage partition is the right one - the
  // flag write just has to happen. Same step the probe, audit-targets and
  // audit-contrast take (9.49), for the same reason.
  await page.evaluate(() => {
    try { localStorage.setItem('switchpost-onboarded', 'true'); } catch {}
  });
  const skipTour = page.getByRole('button', { name: /^skip$|skip the|skip tour/i }).first();
  if (await skipTour.isVisible({ timeout: 2000 }).catch(() => false)) {
    await skipTour.click({ timeout: 3000 }).catch(() => {});
    await page.waitForTimeout(500);
  }

  for (const width of [1440, 375]) {
    await page.setViewportSize({width, height:900});
    // — Composer deep states —
    await page.goto(`${BASE}/posts`, {waitUntil:'load'}); await page.waitForTimeout(1500);
    if (await findBtn(page, '+ New Post')) {
      await page.waitForTimeout(1800);
      await audit(page, `composer@${width}`);
      // every footer toolbar tab that opens a panel
      for (const label of ['Schedule', 'AI', 'Preview', 'Tags']) {
        const ok = await page.evaluate((L) => {
          const d = document.querySelector('[role=dialog]'); if (!d) return false;
          const b = [...d.querySelectorAll('button')].find((x)=>(x.textContent||'').trim().includes(L));
          if (b) { b.click(); return true; } return false;
        }, label);
        await page.waitForTimeout(1000);
        await audit(page, `composer:${label}@${width}`);
        if (label==='Tags' && ok) {
          // tag creation inline form
          const t = await page.evaluate(() => {
            const d=document.querySelector('[role=dialog]'); if(!d) return false;
            const i=d.querySelector('input[placeholder*="tag" i], input[placeholder*="Tag" i]');
            if (i) { i.focus(); return true; } return false;
          });
          if (t) await audit(page, `composer:Tags:input@${width}`);
        }
      }
      // channel selector panel
      const ch = await page.evaluate(() => {
        const d=document.querySelector('[role=dialog]'); if(!d) return false;
        const b=[...d.querySelectorAll('button')].find(x=>/channel/i.test(x.textContent||'') && x.offsetParent);
        if(b){b.click();return true;} return false;
      });
      await page.waitForTimeout(1000);
      await audit(page, `composer:channel@${width}`);
      await page.keyboard.press('Escape'); await page.waitForTimeout(800);
    }
    // — Post detail —
    await page.goto(`${BASE}/posts`, {waitUntil:'load'}); await page.waitForTimeout(1200);
    const opened = await page.evaluate(() => {
      const el=[...document.querySelectorAll('a[href*="/posts/"], [data-post-id]')].find(e=>e.offsetParent);
      if (el) { el.click(); return true; } return false;
    });
    await page.waitForTimeout(1200);
    await audit(page, `post-detail@${width}`);

    // — Kanban card detail —
    await page.goto(`${BASE}/kanban`, {waitUntil:'load'}); await page.waitForTimeout(1500);
    const card = await page.evaluate(() => {
      const el=[...document.querySelectorAll('[draggable=true], .kanban-card, a[href*="campaign"]')].find(e=>e.offsetParent && e.getBoundingClientRect().height>20);
      if (el) { el.click(); return true; } return false;
    });
    await page.waitForTimeout(1000);
    await audit(page, `kanban-card@${width}`);

    // — Calendar: open a day popover (week view default) —
    await page.goto(`${BASE}/calendar`, {waitUntil:'load'}); await page.waitForTimeout(1500);
    await page.evaluate(() => {
      const el=[...document.querySelectorAll('button, [role=button]')].find(e=>e.offsetParent && e.closest('table, [class*=grid]') && e.textContent.trim().match(/^\d+$/));
      if (el) el.click();
    });
    await page.waitForTimeout(1000);
    await audit(page, `calendar-day@${width}`);

    // — Media upload dialog —
    await page.goto(`${BASE}/media`, {waitUntil:'load'}); await page.waitForTimeout(1200);
    const up = await findBtn(page, 'Upload');
    if (up) { await page.waitForTimeout(1000); await audit(page, `media-upload@${width}`); }

    // — Search page with query —
    await page.goto(`${BASE}/search`, {waitUntil:'load'}); await page.waitForTimeout(1000);
    const hasSearch = await page.evaluate(() => !!document.querySelector('input[type=search], input[placeholder*=Search i]'));
    if (hasSearch) {
      await page.fill('input[type=search], input[placeholder*=Search i]', 'the');
      await page.waitForTimeout(1200);
      await audit(page, `search-results@${width}`);
    }

    // — Feed detail / comments —
    await page.goto(`${BASE}/feed`, {waitUntil:'load'}); await page.waitForTimeout(1500);
    const fcard = await page.evaluate(() => {
      const el=[...document.querySelectorAll('a, button')].find(e=>e.offsetParent && /comment|reply|open/i.test(e.textContent||'') && e.closest('main, [class*=feed]'));
      if (el) { el.click(); return true; } return false;
    });
    await page.waitForTimeout(1000);
    await audit(page, `feed-comment@${width}`);

    // — Channels: connect dialog (X tile) —
    await page.goto(`${BASE}/channels`, {waitUntil:'load'}); await page.waitForTimeout(1500);
    const conn = await page.evaluate(() => {
      const el=[...document.querySelectorAll('button')].find(e=>e.offsetParent && /connect|link/i.test(e.textContent||''));
      if (el) { el.click(); return true; } return false;
    });
    await page.waitForTimeout(1200);
    await audit(page, `channels-connect@${width}`);

    // — Automation: new rule dialog —
    await page.goto(`${BASE}/automation`, {waitUntil:'load'}); await page.waitForTimeout(1200);
    const rule = await findBtn(page, 'New');
    if (rule) { await page.waitForTimeout(1200); await audit(page, `automation-new@${width}`); }

    // — Settings modals: webhooks new, signatures new —
    await page.goto(`${BASE}/settings/webhooks`, {waitUntil:'load'}); await page.waitForTimeout(1200);
    if (await findBtn(page, 'New')) { await page.waitForTimeout(1000); await audit(page, `webhook-new@${width}`); }
    await page.goto(`${BASE}/settings/signatures`, {waitUntil:'load'}); await page.waitForTimeout(1200);
    if (await findBtn(page, 'New')) { await page.waitForTimeout(1000); await audit(page, `signature-new@${width}`); }
  }

  await browser.close();

  const seen = new Set();
  const uniq = findings.filter((f) => {
    const k = `${f.where}|${f.kind}|${f.desc.slice(0,40)}`;
    if (seen.has(k)) return false; seen.add(k); return true;
  });
  for (const f of uniq) console.log(`${f.where}  [${f.kind}]  ${f.desc}`);
  console.log(`\n${uniq.length} finding(s)`);
  process.exit(uniq.length ? 1 : 0);
}
main().catch((e) => { console.error(e); process.exit(3); });
