// The F3 drag e2e - the calendar's drag-to-reschedule, driven for real.
//
// The plan's F3 gate is "drag e2e happy-path", and there was NO e2e for
// it: the probe only renders a page, so the drag was never exercised.
// A drag that silently did nothing would have passed every other gate
// in this repo.
//
// The happy path: drag a DRAFT post from one day to another in the month
// view, and assert the post's scheduled time actually moved - to the
// RIGHT INSTANT.
//
// The timezone is the point. The dropped day and time are a WALL CLOCK
// in the APP's selected zone, and the server receives a UTC instant
// (AGENTS.md §9.17). The test seeds a zone that is NOT the container's,
// so a test that computed the expected value with the container's clock
// would pass a broken conversion.
//
// Run: node scripts/e2e-calendar-drag.mjs --url https://localhost:6544 --password "$APP_PASSWORD"
//
// It exits 3 (BLOCKED) when the browser is missing, 1 when the drag
// does not work.

import { chromium } from 'playwright-core';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

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
// A zone the container is NOT in, so the conversion is actually tested.
const ZONE = arg('zone', 'America/New_York');
// A marker unique to THIS RUN, so exactly one chip can match it.
//
// A fixed marker is not enough: the test creates a probe post on every
// run, and they all sit in the calendar together. The cell finder then
// had several candidates to choose from, and it chose one whose post
// was NOT the one the expected instant was worked out from - so the
// test compared one post's schedule against another's.
const PROBE_MARKER = `drag e2e probe ${Date.now()}`;

async function main() {
  if (!PASSWORD) {
    console.error('BLOCKED: no --password');
    process.exit(3);
  }
  const executablePath = findChrome();
  if (!executablePath) {
    console.error('BLOCKED: no Chromium found. Set CHROME_PATH.');
    process.exit(3);
  }

  const browser = await chromium.launch({ executablePath, args: ['--no-sandbox'] });
  const ctx = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
    ignoreHTTPSErrors: true,
  });
  // The app reads its zone from localStorage at boot (timezone.svelte.ts).
  // Seeding it through an init script means the store sees it from its
  // very first read.
  await ctx.addInitScript((zone) => {
    try {
      localStorage.setItem('switchpost-timezone', zone);
    } catch {}
  }, ZONE);

  const page = await ctx.newPage();

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
    document.querySelector('input[type=password]').closest('form').requestSubmit();
  });
  await page.waitForTimeout(2500);

  // Clear out the probe posts from earlier runs.
  //
  // A month cell renders at most THREE chips (`dayEvents.slice(0, 3)`),
  // so a day that has accumulated probe posts hides the newest one
  // behind them - and the test could not find its own chip.
  const cleaned = await page.evaluate(async () => {
    const r = await fetch('/api/posts?limit=200');
    const b = await r.json();
    const stale = (b.posts || []).filter((p) =>
      (p.content || '').startsWith('drag e2e probe')
    );
    for (const p of stale) {
      await fetch(`/api/posts/${p.id}`, { method: 'DELETE' });
    }
    return stale.length;
  });
  if (cleaned) console.log(`  removed ${cleaned} probe post(s) from earlier runs`);

  // A post scheduled in the FUTURE, so there is something to drag.
  //
  // A chip in a PAST cell is not draggable (`draggable={event.state !==
  // 'published' && !past}`) and a past cell refuses the drop
  // (`isPast(date, timezone.value)`), so the test needs a post on a
  // future day. The seeded instance has none - every scheduled post is
  // in the past - so the test makes one.
  await page.goto(`${BASE_URL}/`, { waitUntil: 'load' });
  await page.waitForTimeout(1500);
  const created = await page.evaluate(async (marker) => {
    // Pick a provider that can actually PUBLISH A POST - not the first row.
    //
    // AGENTS.md 9.20: `/api/providers` publishes `char_limit` per provider,
    // and a provider whose limit is 0 (GitHub, Gmail, Drive) has no "publish
    // a post" surface at all. This gate took `integrations[0]`, which on the
    // operator's instance is GitHub - so the fixture POST was refused with
    //   "Content is 19 chars, max 0 for GitHub"
    // and the whole `make audit` run died with exit 3 (BLOCKED) while every
    // other gate was green. The calendar drag itself was never measured.
    //
    // The same rule the composer reads, so the two cannot drift.
    const [integs, providers] = await Promise.all([
      fetch('/api/integrations').then((r) => r.json()),
      fetch('/api/providers').then((r) => r.json()),
    ]);
    const list = Array.isArray(providers) ? providers : (providers.providers ?? []);
    const publishable = new Set(
      list.filter((p) => Number(p.char_limit ?? 0) > 0).map((p) => p.identifier),
    );
    const pick = (integs.integrations ?? []).find(
      (i) => !i.disabled && publishable.has(i.provider_identifier),
    );
    if (!pick) return { err: 'no publishable integration' };
    const id = pick.id;
    const when = new Date(Date.now() + 5 * 86400000).toISOString();
    const r = await fetch('/api/posts', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        integration_ids: [id],
        content: marker,
        scheduled_at: when,
      }),
    });
    const body = await r.json();
    const post = body.posts?.[0];
    if (!post) return { err: `create failed: ${JSON.stringify(body).slice(0, 120)}` };
    return { id: post.id, when, provider: pick.provider_identifier };
  }, PROBE_MARKER);
  if (created.err) {
    console.log(`BLOCKED: ${created.err}`);
    await browser.close();
    process.exit(3);
  }
  console.log(`  seeded a post scheduled ${created.when}`);

  // The month view, so the drop target is a DAY rather than an hour
  // cell.
  await page.goto(`${BASE_URL}/calendar`, { waitUntil: 'load' });
  await page.waitForTimeout(3000);
  // The calendar fetched its range on mount. The probe post was created
  // before the navigation, so it is in the fetch - but the DELETE
  // above and the POST can race the range query, so the page is
  // reloaded to be sure.
  await page.reload({ waitUntil: 'load' });
  await page.waitForTimeout(2500);
  await page.evaluate(() => {
    // The view buttons carry an emoji prefix ("📅 Month"), so the
    // match is on the SUFFIX.
    const b = [...document.querySelectorAll('button')].find((x) =>
      /Month\s*$/.test((x.textContent || '').trim())
    );
    if (b) b.click();
  });
  await page.waitForTimeout(2000);

  // Find a DRAFT chip and a FUTURE day cell to drop it on.
  //
  // A published post is deliberately not draggable, and dropping one
  // raises the "already published" modal - a DIFFERENT path, with its
  // own behaviour.
  //
  // The target must not be in the PAST: `isPast(date, timezone.value)`
  // gates the drop, and a past cell refuses it.
  const viewState = await page.evaluate(() => ({
    zone: localStorage.getItem('switchpost-timezone'),
    dataDateCells: document.querySelectorAll('[data-date]').length,
    dataDropCells: document.querySelectorAll('[data-drop-date]').length,
    gridcells: document.querySelectorAll('[role="gridcell"]').length,
    buttons: [...document.querySelectorAll('button')]
      .map((b) => (b.textContent || '').trim())
      .filter((t) => /Month|Week|Day|List/.test(t)),
  }));
  console.log('  view: ' + JSON.stringify(viewState));

  // The calendar's own request, for the diagnostic when the chip is
  // not there. A broken zone conversion changes the RANGE the calendar
  // asks for, so the post never arrives - and the request URL says so
  // in one line.
  const calRequests = [];
  page.on('request', (req) => {
    if (req.url().includes('/api/calendar')) {
      calRequests.push(decodeURIComponent(req.url()).replace(BASE_URL, ''));
    }
  });

  const target = await page.evaluate((marker) => {
    const cells = [...document.querySelectorAll('[data-date]')];
    // The LOCAL date, not the UTC one: `formatDateKey` builds the cell
    // key from `getFullYear()/getMonth()/getDate()`, so the cell keys
    // are the BROWSER's local dates. Comparing them against the UTC
    // date is off by a day for half the world - and then every cell
    // reads as "past" or "future" wrongly.
    const now = new Date();
    const today = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
    const future = cells.filter((c) => (c.getAttribute('data-date') || '') > today);
    if (!future.length) return { err: 'no future cell' };

    for (const cell of cells) {
      // The chip's TEXT is truncated to fit the month cell, so the
      // marker is matched against its aria-label, which carries the
      // full content.
      const chip = [...cell.querySelectorAll('div[draggable="true"]')].find((c) =>
        ((c.getAttribute('aria-label') || '') + ' ' + (c.textContent || '')).includes(
          marker
        )
      );
      if (!chip) continue;
      const from = cell.getAttribute('data-date');
      // A DIFFERENT future day, so the move is observable.
      const to = future.find((c) => c.getAttribute('data-date') !== from);
      if (!to) continue;
      return {
        chipText: (chip.textContent || '').trim().slice(0, 40),
        // The chip carries no id in the DOM, so the id comes from the
        // calendar's own data. The drop PUTs to /api/posts/:id/date, and
        // the test reads the id back from the request it captured.
        from,
        to: to.getAttribute('data-date'),
      };
    }
    return { err: 'no draggable chip for the probe post' };
  }, PROBE_MARKER);

  if (target.err) {
    // A FAILURE, not a BLOCKED.
    //
    // BLOCKED means "the environment could not run this" - no browser,
    // no password. A missing chip is a statement about the APP: the
    // test made a post, the calendar was asked for the month, and the
    // post was not in the result. That is exactly what a broken zone
    // conversion looks like, because it moves the range the calendar
    // asks for - so it has to FAIL, or `make audit` would pass on a
    // build whose calendar cannot show a post it just made.
    const days = await page.evaluate(async () => {
      const r = await fetch(
        '/api/calendar?start=2026-09-01T00:00:00.000Z&end=2026-11-30T00:00:00.000Z'
      );
      const b = await r.json();
      return (b.days || []).map((d) => d.date);
    });
    console.log(`  ✗ ${target.err}`);
    console.log(
      `    the calendar asked for: ${calRequests.join(' | ') || '(nothing)'}`
    );
    console.log(`    the API returned these days: ${days.join(', ') || '(none)'}`);
    await browser.close();
    process.exit(1);
  }

  // The post's CURRENT scheduled time, so the expected instant can be
  // worked out: the drop keeps the wall-clock TIME and moves the DAY.
  const before = await page.evaluate(async (id) => {
    const r = await fetch(`/api/posts/${id}`);
    const b = await r.json();
    return { scheduled_at: b.post?.scheduled_at ?? b.scheduled_at ?? null };
  }, created.id);
  if (!before.scheduled_at) {
    console.log('BLOCKED: could not read the post it was going to drag');
    await browser.close();
    process.exit(3);
  }

  // The API call the drop makes.
  const calls = [];
  page.on('request', (req) => {
    if (req.method() === 'PUT' && /\/api\/posts\/[^/]+\/date/.test(req.url())) {
      calls.push({ url: req.url(), body: req.postData() });
    }
  });

  // HTML5 drag-and-drop: dispatch the events the browser would, with
  // ONE DataTransfer carried across all three.
  const didDrag = await page.evaluate(
    ({ from, to }) => {
      const source = document.querySelector(`[data-date="${from}"]`);
      const targetCell = document.querySelector(`[data-date="${to}"]`);
      if (!source || !targetCell) return { err: 'the cells vanished' };
      const chip = source.querySelector('div[draggable="true"]');
      if (!chip) return { err: 'no chip in the source cell' };

      const dt = new DataTransfer();
      chip.dispatchEvent(
        new DragEvent('dragstart', { bubbles: true, cancelable: true, dataTransfer: dt })
      );
      targetCell.dispatchEvent(
        new DragEvent('dragover', {
          bubbles: true,
          cancelable: true,
          dataTransfer: dt,
        })
      );
      targetCell.dispatchEvent(
        new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: dt })
      );
      chip.dispatchEvent(
        new DragEvent('dragend', { bubbles: true, cancelable: true, dataTransfer: dt })
      );
      return { ok: true };
    },
    target
  );

  if (didDrag.err) {
    console.log(`FAILED: ${didDrag.err}`);
    await browser.close();
    process.exit(1);
  }

  await page.waitForTimeout(2500);

  const findings = [];

  if (calls.length === 0) {
    findings.push('the drop made NO reschedule call - the drag did nothing');
  } else {
    const body = JSON.parse(calls[0].body || '{}');
    console.log(`  dragged "${target.chipText}" ${target.from} -> ${target.to}`);
    console.log(`  PUT ${calls[0].url.replace(BASE_URL, '')} ${calls[0].body}`);

    if (!body.scheduled_at) {
      findings.push('the reschedule call carried no scheduled_at');
    } else {
      // The wall clock the post was dropped on, read in the APP's zone.
      //
      // The chip's own time is the time the post already had; the drop
      // keeps it unless the view supplied an hour. The month view has
      // no hour, so the post keeps its time and moves to the new DAY.
      //
      // The expected instant is that wall clock in the seeded zone.
      // The expected instant.
      //
      // The drop keeps the post's WALL-CLOCK TIME and moves it to the
      // new DAY. So the expected instant is: the original scheduled_at
      // read in the ZONE (which gives the wall clock), with the DATE
      // replaced by the target day.
      //
      // The first version of this parsed the wall clock in the
      // BROWSER's zone and re-formatted it into ZONE, which returns the
      // instant it started with - so it could never detect a
      // wrong-zone conversion, which is the ONLY thing it exists to
      // detect. The wall clock is now read out of the original instant
      // IN the zone, which is the only reading that says anything.
      const expected = (() => {
        const wall = new Intl.DateTimeFormat('en-US', {
          timeZone: ZONE,
          year: 'numeric',
          month: '2-digit',
          day: '2-digit',
          hour: '2-digit',
          minute: '2-digit',
          second: '2-digit',
          hour12: false,
        })
          .formatToParts(new Date(before.scheduled_at))
          .reduce((acc, p) => ({ ...acc, [p.type]: p.value }), {});
        const hh = Number(wall.hour) % 24;
        const mm = Number(wall.minute);
        // The wall clock is a WALL CLOCK: it is HH:MM, with no
        // seconds. `wallClockToUtc` takes HH:MM, so the seconds are
        // dropped - and the expected instant has to drop them too, or
        // it is off by the post's sub-minute remainder.
        const ss = 0;
        const [y, m, d] = target.to.split('-').map(Number);

        // The wall clock AS IF it were UTC, then the zone's offset AT
        // THAT wall clock subtracted.
        const wallAsUtc = Date.UTC(y, m - 1, d, hh, mm, ss);
        const fmt = new Intl.DateTimeFormat('en-US', {
          timeZone: ZONE,
          year: 'numeric',
          month: '2-digit',
          day: '2-digit',
          hour: '2-digit',
          minute: '2-digit',
          second: '2-digit',
          hour12: false,
        });
        const parts = Object.fromEntries(
          fmt.formatToParts(new Date(wallAsUtc)).map((p) => [p.type, p.value])
        );
        const asSeenInZone = Date.UTC(
          Number(parts.year),
          Number(parts.month) - 1,
          Number(parts.day),
          Number(parts.hour) % 24,
          Number(parts.minute),
          Number(parts.second)
        );
        // Positive when the zone is AHEAD of UTC.
        const offsetMs = asSeenInZone - wallAsUtc;
        return { expected: new Date(wallAsUtc - offsetMs).toISOString() };
      })();

      if (expected.err) {
        findings.push(`could not work out the expected instant: ${expected.err}`);
      } else {
        const got = new Date(body.scheduled_at).toISOString();
        if (got !== expected.expected) {
          findings.push(
            `the wall clock was converted in the WRONG zone: the drop sent ${got}, and ${target.to} in ${ZONE} is ${expected.expected}`
          );
        } else {
          console.log(`  the wall clock converted correctly: ${got}`);
        }
      }
    }
  }

  // And the post must be on the NEW day after a reload.
  if (calls.length > 0) {
    await page.reload({ waitUntil: 'load' });
    await page.waitForTimeout(2500);
    await page.evaluate(() => {
      const b = [...document.querySelectorAll('button')].find((x) =>
        /Month\s*$/.test((x.textContent || '').trim())
      );
      if (b) b.click();
    });
    await page.waitForTimeout(2500);
    // The post's day, from the API.
    //
    // The DOM cannot answer it: a month cell renders at most THREE
    // chips and folds the rest into a "+1 more" row, so a cell with
    // four posts does not show the fourth one's label at all.
    const landed = await page.evaluate(async (id) => {
      const r = await fetch(`/api/posts/${id}`);
      const b = await r.json();
      return {
        date: b.post?.scheduled_at ?? b.scheduled_at ?? null,
      };
    }, created.id);
    // The post is on the target day when its scheduled instant, read in
    // the app's zone, falls on that day.
    const landedDay = await page.evaluate(
      ({ iso, zone }) => {
        if (!iso) return null;
        const parts = new Intl.DateTimeFormat('en-CA', {
          timeZone: zone,
          year: 'numeric',
          month: '2-digit',
          day: '2-digit',
        }).formatToParts(new Date(iso));
        const g = (t) => parts.find((p) => p.type === t)?.value ?? '';
        return `${g('year')}-${g('month')}-${g('day')}`;
      },
      { iso: landed.date, zone: ZONE }
    );
    if (landedDay !== target.to) {
      findings.push(
        `after a reload the post is on ${landedDay}, not ${target.to} - the move did not stick`
      );
    } else {
      console.log(`  the post is on ${target.to} after a reload`);
    }
  }

  // The probe post must not be left behind.
  await page.evaluate(async () => {
    const r = await fetch('/api/posts?limit=200');
    const b = await r.json();
    for (const p of (b.posts || []).filter((x) =>
      (x.content || '').startsWith('drag e2e probe')
    )) {
      await fetch(`/api/posts/${p.id}`, { method: 'DELETE' });
    }
  });

  await browser.close();

  if (findings.length) {
    for (const f of findings) console.log(`  ✗ ${f}`);
    console.log(`\n${findings.length} drag failure(s)`);
    process.exit(1);
  }
  console.log('\n0 drag failures');
  process.exit(0);
}

main().catch((e) => {
  console.error(e);
  process.exit(3);
});
