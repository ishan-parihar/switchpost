// A browser-side audit of the REAL rendered targets.
//
// The static probe reads the DOM at three widths, but it only visits the
// page's default state. The composer, the modals and the popovers are
// only reachable by interaction, and their controls are where the small
// targets hide.
//
// This walks the app the way the founder does: open the composer, open
// each panel, and report every control whose rendered box is under the
// 44px floor.
//
// Run: node scripts/audit-targets.mjs --url https://localhost:6544 --password "$APP_PASSWORD"
//
// It exits 3 (BLOCKED) when the browser is missing, and 1 when a target
// is under the floor, so it can be a gate.

import { chromium } from 'playwright-core';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// The same Chromium search the probe uses. The playwright-core in this
// repo expects a browser build that is not installed, so the path has to
// be found the way `probe.mjs` finds it.
const CHROME_CANDIDATES = [
  process.env.CHROME_PATH,
  ...fs
    .readdirSync(path.join(os.homedir(), '.cache/ms-playwright'), { withFileTypes: true })
    .filter((d) => d.isDirectory() && d.name.startsWith('chromium-'))
    .flatMap((d) => [
      path.join(os.homedir(), '.cache/ms-playwright', d.name, 'chrome-linux64/chrome'),
      path.join(os.homedir(), '.cache/ms-playwright', d.name, 'chrome-linux/chrome'),
    ]),
  '/usr/bin/chromium',
  '/usr/bin/chromium-browser',
  '/usr/bin/google-chrome',
].filter(Boolean);

function findChrome() {
  for (const p of CHROME_CANDIDATES) {
    if (fs.existsSync(p)) return p;
  }
  return null;
}

const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > -1 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
};

const BASE_URL = arg('url', 'https://localhost:6544');
const PASSWORD = arg('password', '');
const OUT = arg('out', '');

const FLOOR = 44;

function FINDINGS() {
  const out = [];
  const warn = [];
  document.querySelectorAll('button, a[href], [role="button"], input, select, textarea, [role="checkbox"], [role="tab"]').forEach((el) => {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') return;
    if (cs.opacity === '0') return;
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return;
    // A control that is only in the DOM and not rendered is not a target.
    //
    // SEVERITY matches the standard and the probe (AGENTS.md 9.37,
    // WCAG 2.5.8 Target Size Minimum): below 24x24 is a FAIL, 24-44 is
    // a WARNING. The audit used to treat anything under 44 as a FAIL,
    // so a control the probe classified as a warning failed the gate
    // here - two gates disagreeing about the same rendered box.
    if (r.height >= 24 && r.width >= 24) {
      if (r.height >= 44 && r.width >= 44) return;
      warn.push({
        tag: el.tagName.toLowerCase(),
        role: el.getAttribute('role') || '',
        name: (el.getAttribute('aria-label') || el.textContent || el.getAttribute('placeholder') || '').trim().slice(0, 40),
        w: Math.round(r.width),
        h: Math.round(r.height),
        cls: String(el.className).slice(0, 90),
      });
      return;
    }
    // A VISUALLY HIDDEN control is not a target either.
    //
    // The skip link is the case: it is clipped to 1x1 until it takes the
    // focus, which is the correct pattern - a keyboard user Tab's to it
    // and it becomes visible. In its resting state it is a 1x1 target,
    // and reporting that is reporting a violation of the PATTERN.
    //
    // The test is the COMBINATION: a 1x1 box that is clipped away (or
    // pushed off-screen by a huge negative text-indent). A 1x1 box on
    // its own is a real control that happens to be tiny.
    const clippedAway =
      (cs.clip && cs.clip !== 'auto' && /rect\(\s*0(px)?\s*,\s*0(px)?\s*,\s*0(px)?\s*,\s*0(px)?\s*\)/.test(cs.clip)) ||
      (cs.clipPath && cs.clipPath !== 'none') ||
      parseFloat(cs.textIndent) < -100;
    if (clippedAway && r.width <= 2 && r.height <= 2) return;
    // A control in the 24-44 band inside a 44px label clears the FAIL
    // band via the wrapper (checkboxes): the wrapper is the real
    // target, and the 9.37 warning band applies to the same box.
    const wrapper = el.closest('label');
    if (wrapper) {
      const wr = wrapper.getBoundingClientRect();
      if (wr.height >= 24 && wr.width >= 24) {
        if (wr.height >= 44 && wr.width >= 44) return;
        warn.push({
          tag: el.tagName.toLowerCase(),
          role: el.getAttribute('role') || '',
          name: (el.getAttribute('aria-label') || el.textContent || el.getAttribute('placeholder') || '').trim().slice(0, 40),
          w: Math.round(r.width),
          h: Math.round(r.height),
          cls: String(el.className).slice(0, 90),
        });
        return;
      }
    }
    out.push({
      tag: el.tagName.toLowerCase(),
      role: el.getAttribute('role') || '',
      name: (el.getAttribute('aria-label') || el.textContent || el.getAttribute('placeholder') || '').trim().slice(0, 40),
      w: Math.round(r.width),
      h: Math.round(r.height),
      cls: String(el.className).slice(0, 90),
    });
  });
  return { fails: out, warns: warn };
}

// The ENTRY ANIMATION must have finished before the measurement.
//
// Every page fades in over 0.2s (`.page-enter`), and the fade is an
// `opacity` animation on an ANCESTOR of the text. A contrast measure
// treats an ancestor with `opacity < 1` as a translucent group and
// stops the backdrop walk there - so a page measured mid-fade was
// measured against the wrong background. A page that ANIMATES forever
// (the skeleton shimmer) would hang, so only the finite ones are
// awaited.
async function settleAnimations(page) {
  await page
    .evaluate(async () => {
      const anims = document.getAnimations().filter((a) => {
        const t = a.effect?.getComputedTiming?.();
        return t && t.iterations !== Infinity;
      });
      if (!anims.length) return;
      await Promise.all(anims.map((a) => a.finished.catch(() => undefined)));
    })
    .catch(() => {});
}

async function main() {
  if (!PASSWORD) {
    console.error('BLOCKED: no --password');
    process.exit(3);
  }

  let browser;
  try {
    const executablePath = findChrome();
    if (!executablePath) {
      console.error('BLOCKED: no Chromium found. Set CHROME_PATH.');
      process.exit(3);
    }
    browser = await chromium.launch({ executablePath, args: ['--no-sandbox'] });
  } catch (e) {
    console.error(`BLOCKED: no browser: ${e.message}`);
    process.exit(3);
  }

  // The context carries the certificate exemption, exactly as the
  // probe does: the instance serves a self-signed certificate.
  const ctx = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    ignoreHTTPSErrors: true,
  });
  const page = await ctx.newPage();
  const findings = [];
  const warnings = [];

  const record = async (where) => {
    const { fails, warns } = await page.evaluate(FINDINGS);
    for (const r of fails) findings.push({ where, ...r });
    for (const r of warns) warnings.push({ where, ...r });
  };

  // Log in.
  await page.goto(`${BASE_URL}/login`, { waitUntil: 'load' });
    // The password field is client-rendered (adapter-static SPA shell), so
  // wait for it and report BLOCKED if the app never comes up instead of
  // timing out inside fill() - the same guard state-hunt.mjs carries.
  try {
    await page.waitForSelector('input[type=password]', { timeout: 20000 });
  } catch {
    console.error('BLOCKED: login page never rendered a password field');
    process.exit(3);
  }
  await page.fill('input[type=password]', PASSWORD);
  await page.evaluate(() => {
    const i = document.querySelector('input[type=password]');
    i.closest('form').requestSubmit();
  });
  await page.waitForTimeout(2500);

  // ── THE ONBOARDING TOUR, and it was on EVERY route this gate measured ──
  //
  // MEASURED 2026-10-31: replayed this gate's exact login with no
  // dismissal and the tour `Welcome to Switchpost` was open on /channels,
  // /posts/new, /calendar and / alike - one full-screen `role="dialog"`,
  // and its own Skip button is one of the controls this gate counted.
  //
  // So every tap target below was measured through an overlay that
  // changes the page's DOM, and the tour's own buttons were in the
  // population. "0 controls under the floor" was a measurement of the
  // tour and the page it covers, not of the app the founder uses.
  //
  // Same step `audit-dialogs.mjs` and `audit-contrast.mjs` take, for the
  // same reason (9.49): set the flag so the layout does not raise it again
  // on the next navigation, then click Skip if the tour is on screen. It
  // has to be AFTER the login - the layout only raises the tour once the
  // app is authenticated, so dismissing before it does nothing.
  await page.evaluate(() => {
    try { localStorage.setItem('switchpost-onboarded', 'true'); } catch {}
  });
  const skipTour = page.getByRole('button', { name: /^skip$|skip the|skip tour/i }).first();
  if (await skipTour.isVisible({ timeout: 2000 }).catch(() => false)) {
    await skipTour.click({ timeout: 3000 }).catch(() => {});
    await page.waitForTimeout(500);
  }

  const ROUTES = [
    '/', '/calendar', '/posts', '/campaigns', '/channels', '/analytics',
    '/activity', '/research', '/feed', '/media', '/tags', '/kanban',
    '/comments', '/dms', '/search', '/automation', '/settings',
    '/settings/profile', '/settings/notifications', '/settings/signatures',
    '/settings/webhooks', '/settings/rss', '/settings/mcp', '/settings/developer',
  ];

  // The founder's phone widths, not just the desktop: the mobile
  // layouts are DIFFERENT CSS (collapsed sidebars, stacked toolbars),
  // and a control that clears the floor at 1440 can be squeezed by
  // its mobile layout. The probe measures 375/360 for overflow and
  // contrast; this gate now measures them for tap targets too.
  for (const width of [1440, 375, 360]) {
    await page.setViewportSize({ width, height: 900 });
    for (const route of ROUTES) {
      await page.goto(`${BASE_URL}${route}`, { waitUntil: 'load' });
      await page.waitForTimeout(1200);
      await settleAnimations(page);
      await record(`${route}@${width}`);
    }

    // The composer, and the panels inside it.
    await page.goto(`${BASE_URL}/posts/new`, { waitUntil: 'load' });
    await page.waitForTimeout(1500);
    const opened = await page.evaluate(() => {
      const b = [...document.querySelectorAll('button')].find((x) => (x.textContent || '').trim() === '+ New Post');
      if (b) { b.click(); return true; }
      return false;
    });
    if (opened) {
      await page.waitForTimeout(1500);
      await record(`composer@${width}`);

      // Each collapsible section in the composer.
      for (const label of ['🧵 Thread Builder ▸', 'AI', 'Preview']) {
        await page.evaluate((L) => {
          const d = document.querySelector('[role=dialog]');
          if (!d) return;
          const b = [...d.querySelectorAll('button')].find((x) => (x.textContent || '').trim() === L);
          if (b) b.click();
        }, label);
        await page.waitForTimeout(900);
        await record(`composer:${label}@${width}`);
      }
    }
  }

  await browser.close();

  // Dedupe by class + size + name, so one control in three themes is one finding.
  const seen = new Set();
  const dedupe = (list) => list.filter((f) => {
    const k = `${f.where}|${f.tag}|${f.name}|${f.w}x${f.h}|${f.cls}`;
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
  const unique = dedupe(findings);
  const uniqueWarns = dedupe(warnings);

  for (const f of unique) {
    console.log(`${f.where}  ${f.w}x${f.h}  <${f.tag}${f.role ? ` ${f.role}` : ''}> "${f.name}"  ${f.cls}`);
  }
  console.log(`\n${unique.length} control(s) under the 24x24 floor`);
  if (uniqueWarns.length) {
    console.log(`${uniqueWarns.length} control(s) in the 24-44 warning band (reported, not gating):`);
    for (const f of uniqueWarns.slice(0, 10)) {
      console.log(`  ${f.where}  ${f.w}x${f.h}  "${f.name}"`);
    }
    if (uniqueWarns.length > 10) console.log(`  ... and ${uniqueWarns.length - 10} more`);
  }

  if (OUT) {
    const { writeFileSync } = await import('node:fs');
    writeFileSync(OUT, JSON.stringify({ fails: unique, warnings: uniqueWarns }, null, 2));
  }

  process.exit(unique.length ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(3);
});
