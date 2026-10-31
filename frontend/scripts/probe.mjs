#!/usr/bin/env node
// ─── Website-design probe (AUD-9 gate) ───────────────────────
//
// Loads each dashboard route in a real Chromium at three widths and
// reports two severities:
//
//   FAIL  — a defect a visitor would hit: an uncaught page error, an
//           HTTP 5xx, horizontal overflow, text below the WCAG AA
//           contrast floor, a broken heading outline, or overlapping
//           text that is not a clipping artifact.
//   WARN  — advisory: small tap targets and sub-10px labels.
//
// The gate is 0 FAIL. Warnings are reported, not gated.
//
// Usage:
//   node scripts/probe.mjs [--url http://localhost:6543] [--password pw]
//                          [--widths 1440,768,375] [--theme dark,light]
//                          [--out probe-out]
//
// Requires playwright-core (a devDependency). Chromium is resolved from
// PLAYWRIGHT_BROWSERS_PATH (~/.cache/ms-playwright) or the system path;
// there is no `playwright install` step in CI, so the cached browser is
// used directly.

import { chromium } from 'playwright-core';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { CONTRAST_CORE } from './contrast-core.mjs';


// ── CLI ──────────────────────────────────────────────────────

const argv = process.argv.slice(2);
const flag = (name, dflt) => {
  const i = argv.indexOf(`--${name}`);
  return i === -1 ? dflt : argv[i + 1] ?? dflt;
};

const BASE_URL = flag('url', process.env.PROBE_URL || 'http://localhost:6543');
const PASSWORD = flag('password', process.env.APP_PASSWORD || '');
// Skip the login and probe the routes UNAUTHENTICATED - the only way to
// measure /login, because the layout guard redirects a logged-in
// visitor to '/' and an authenticated probe of /login measures the
// dashboard twice. Used by make audit's dedicated login pass.
const LOGGED_OUT = argv.includes('--logged-out');
// 360 is the NARROWEST width the plan's F5 gate names ("a 360px mobile
// pass on composer/calendar/channels"). It was missing from the list, so
// the plan's own gate was never run.
const WIDTHS = flag('widths', '1440,768,375,360').split(',').map(Number);
const THEMES = flag('theme', 'dark,light').split(',');
const OUT_DIR = flag('out', 'probe-out');

// The manual suite's routes. Kept to the surfaces this dashboard owns —
// no team/workspace/billing/admin/agent-chat/video pages exist to probe.
//
// Every route the app ships is here. The 2026-10-20 red-team found 16
// pages that had NEVER been rendered: /kanban, /comments, /dms, /search,
// /automation, /campaigns/[id], /posts/[id] and the eight /settings/*
// sub-pages. A page that is never rendered is a page whose defects are
// never found, so the list is the app's route table, not a sample of it.
//
// The two dynamic routes need an id. They are discovered from the API at
// run time (see RESOLVED_ROUTES) because the ids are per-instance.
const STATIC_ROUTES = flag(
  'routes',
  [
    '/',
    '/calendar',
    '/posts',
    '/posts/new',
    '/campaigns',
    '/channels',
    '/analytics',
    '/activity',
    '/research',
    '/feed',
    '/media',
    '/tags',
    '/settings',
    '/settings/profile',
    '/settings/notifications',
    '/settings/signatures',
    '/settings/webhooks',
    '/settings/rss',
    '/settings/mcp',
    '/settings/developer',
    '/kanban',
    '/comments',
    '/dms',
    '/search',
    '/automation',
  ].join(',')
).split(',');
// NOTE: /login is NOT in this list, and cannot be: the layout guard
// redirects an authenticated visitor to '/' (+layout.ts), and the
// probe logs in first, so a /login visit here would measure the
// DASHBOARD twice. The login page is probed by a separate logged-out
// pass in `make audit` (--routes /login --logged-out).

// Routes that need a per-instance id, and where to get one.
const RESOLVED_ROUTES = [
  { path: '/campaigns/{id}', from: '/api/campaigns', pick: (rows) => rows[0]?.id },
  { path: '/posts/{id}', from: '/api/posts?limit=50', pick: (rows) => rows.posts?.[0]?.id },
];

const CHROME_CANDIDATES = [
  process.env.CHROME_PATH,
  ...fs
    .readdirSync(path.join(os.homedir(), '.cache/ms-playwright'), {
      withFileTypes: true,
    })
    .filter((d) => d.isDirectory() && d.name.startsWith('chromium-'))
    .flatMap((d) => [
      path.join(
        os.homedir(),
        '.cache/ms-playwright',
        d.name,
        'chrome-linux64/chrome'
      ),
      path.join(
        os.homedir(),
        '.cache/ms-playwright',
        d.name,
        'chrome-linux/chrome'
      ),
    ]),
  '/usr/bin/chromium',
  '/usr/bin/chromium-browser',
  '/usr/bin/google-chrome',
].filter(Boolean);

function findChrome() {
  for (const p of CHROME_CANDIDATES) {
    if (fs.existsSync(p)) return p;
  }
  throw new Error('No Chromium found. Set CHROME_PATH.');
}

// ── In-page audit ────────────────────────────────────────────
//
// Run as a single function in the page so one round-trip collects every
// measurement. Pure DOM reads — no layout mutation.

// The AUDIT body: the shared contrast measurement from contrast-core.mjs
// (§9.18: ONE implementation - probe.mjs, audit-contrast.mjs and state-hunt.mjs
// each carried a private copy until copy #3 shipped a channel-alpha typo) plus
// this script's own report policy. Assembled with new Function because ESM is
// strict mode and a direct eval would scope the helpers to the eval itself.
const AUDIT_BODY = `
${CONTRAST_CORE}
  const fails = [];
  const warns = [];

  // Large text (>=24px, or >=18.66px bold) clears at 3:1; the rest at
  // 4.5:1 — WCAG 1.4.3.
  const requiredRatio = (size, weight) => {
    const bold = Number(weight) >= 700;
    return size >= 24 || (bold && size >= 18.66) ? 3 : 4.5;
  };

  // ── Contrast ──
  for (const el of document.querySelectorAll('*')) {
    if (el.closest('[hidden]') || el.getAttribute('aria-hidden') === 'true')
      continue;
    const cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none') continue;

    // Only elements with their own text run. A wrapper's colour inherits,
    // so measuring it twice duplicates the same pair.
    const own = Array.from(el.childNodes).some(
      (n) =>
        n.nodeType === Node.TEXT_NODE &&
        n.textContent.trim().length > 0 &&
        // Clipped text keeps its full box but its ink is cut off; the
        // check below cannot see the clip, so it would report a false
        // pair for text the visitor never reads.
        !el.classList.contains('truncate')
    );
    if (!own) continue;

    const fg = parseColor(cs.color);
    if (!fg || fg.a < 0.95) continue;

    // A DISABLED control is exempt from the contrast requirement (WCAG
    // 1.4.3 names "incidental" text, and a disabled control is not
    // content the reader can act on). The probe renders the disabled
    // state - a \`disabled:opacity-50\` button measures ~2.5:1 and would
    // be reported forever.
    if (el.disabled || el.closest('[aria-disabled="true"]')) continue;

    // An element that is INVISIBLE is not measured at all. The app
    // hides hover-revealed controls with \`opacity-0\` (the kanban's
    // "⋯" menu, the campaign's "✕", the dashboard's "→"). At opacity 0
    // the ink and the backdrop are the same pixels, so the ratio is
    // 1.00:1 - a perfect reading and a meaningless one. The control
    // becomes visible on hover, and the probe's screenshots are taken
    // without a hover, so this is the state the reader sees.
    const ownOpacity = parseFloat(cs.opacity);
    if (!Number.isNaN(ownOpacity) && ownOpacity < 0.05) continue;

    // An \`opacity\` on the element (or on an ancestor) makes the RENDERED
    // ink lighter than the computed colour. The computed colour is what
    // the stylesheet says; the pixels are what the visitor reads.
    //
    // The calendar's chip time was \`color: rgb(185 28 28)\` with
    // \`opacity: 0.7\` on a 15%-alpha red wash: the computed colour
    // measured 5.00:1, the RENDERED ink measured 3.24:1 - under the
    // floor. The check below read the computed colour and passed it.
    //
    // The opacity chain: \`opacity\` is NOT inherited, but an element
    // with \`opacity < 1\` is composited as a GROUP, so its descendants
    // are faded together. The fade applies to the element's own ink AND
    // to its background - so the backdrop under a faded element is the
    // backdrop under its GROUP, and the element's own background is
    // faded into that same backdrop.
    //
    // So the correct computation is: walk up, and at the first
    // ancestor with \`opacity < 1\`, that ancestor's own background is
    // faded into ITS backdrop, and the text ink is faded the same way.
    const opacity = inheritedOpacity(el);
    const backdrop = effectiveBg(el);
    let fgEff = fg;
    if (opacity < 0.999) {
      fgEff = {
        r: fg.r * opacity + backdrop.r * (1 - opacity),
        g: fg.g * opacity + backdrop.g * (1 - opacity),
        b: fg.b * opacity + backdrop.b * (1 - opacity),
        a: 1,
      };
    }

    const size = parseFloat(cs.fontSize);
    const ratio = contrast(fgEff, backdrop);
    const need = requiredRatio(size, cs.fontWeight);
    if (ratio < need - 0.01) {
      fails.push({
        check: 'contrast',
        detail: \`\${Math.round(size)}px \${ratio.toFixed(2)}:1 < \${need}:1 — "\${el.textContent.trim().slice(0, 40)}"\`,
        selector: el.tagName.toLowerCase() + (el.className ? '.' + String(el.className).split(' ')[0] : ''),
      });
    }
  }

  // ── Horizontal overflow ──
  // A page that scrolls sideways at a phone width is a layout break.
  const de = document.documentElement;
  if (de.scrollWidth > de.clientWidth + 1) {
    fails.push({
      check: 'overflow',
      detail: \`scrollWidth \${de.scrollWidth} > clientWidth \${de.clientWidth}\`,
      selector: 'html',
    });
  }
  for (const el of document.querySelectorAll('body *')) {
    const cs = getComputedStyle(el);
    if (cs.overflowX === 'auto' || cs.overflowX === 'scroll' || cs.overflowX === 'hidden')
      continue;
    if (el.scrollWidth > el.clientWidth + 1 && el.clientWidth > 0) {
      // A scroll container's own overflow is its job, not a defect.
      const canScroll = (n) => {
        while (n && n !== document.body) {
          const s = getComputedStyle(n);
          if (/(auto|scroll)/.test(s.overflowX)) return true;
          n = n.parentElement;
        }
        return false;
      };
      if (!canScroll(el)) {
        fails.push({
          check: 'overflow',
          detail: \`element scrollWidth \${el.scrollWidth} > clientWidth \${el.clientWidth}\`,
          selector: el.tagName.toLowerCase(),
        });
      }
    }
  }

  // ── Heading outline ──
  // One h1 per page (the route title), and no skipped levels — a11y
  // 1.3.1 / 2.4.6, and the AUD-9 rule.
  const heads = Array.from(document.querySelectorAll('h1,h2,h3,h4,h5,h6'));
  const h1s = heads.filter((h) => h.tagName === 'H1');
  if (h1s.length === 0) {
    fails.push({ check: 'heading', detail: 'no <h1> on the page', selector: 'body' });
  } else if (h1s.length > 1) {
    fails.push({ check: 'heading', detail: \`\${h1s.length} <h1> elements\`, selector: 'h1' });
  }
  let prev = 0;
  for (const h of heads) {
    const lvl = Number(h.tagName[1]);
    if (prev && lvl > prev + 1) {
      fails.push({
        check: 'heading',
        detail: \`<h\${prev}> → <h\${lvl}> skips a level — "\${h.textContent.trim().slice(0, 40)}"\`,
        selector: h.tagName.toLowerCase(),
      });
    }
    prev = lvl;
  }

  // ── Overlapping text ──
  // Two visible text runs on the same line whose boxes overlap by more
  // than a hair, when neither is clipped. Absolute/relative/fixed and
  // transformed boxes legitimately overlap, so those are exempt.
  //
  // Occlusion: an element visually hidden behind an opaque overlay (for
  // example page chrome behind a full-screen modal's opaque backdrop) is
  // skipped - elementFromPoint returns the covering element, not the
  // covered one.
  function isVisuallyOccluded(el) {
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return false;
    const pts = [
      [r.left + r.width / 2, r.top + r.height / 2],
      [r.left + r.width * 0.25, r.top + r.height * 0.5],
      [r.left + r.width * 0.75, r.top + r.height * 0.5],
    ];
    for (const [x, y] of pts) {
      const top = document.elementFromPoint(x, y);
      if (!top) continue;
      if (top === el || el.contains(top)) continue;
      // top covers el and is not contained by el -> el is hidden behind it.
      if (!top.contains(el)) {
        const cs = getComputedStyle(top);
        if (cs.visibility !== 'hidden' && cs.display !== 'none') return true;
      }
    }
    return false;
  }

  // Clip a box to every ancestor that actually clips it. An element inside
  // a scroller, an "overflow: hidden" box or a collapsed panel has a rect far
  // larger than the pixels it paints: the composer signature sat at
  // y=731..955 in a 900px viewport while .rich-editor clipped it at 785, so
  // 170px of phantom rect overlapped the footer buttons it never reached.
  // elementFromPoint proved the buttons were still the top hit-targets there - a
  // painting difference, not a layout one. Without this the probe reports
  // FAILs the visitor can never see.
  //
  // A scroller clips to its padding box: from cr.top down clientHeight.
  // Content scrolled out above or below still reports its un-scrolled layout
  // rect, so clipping to the scroller's own visible band is enough - the
  // band itself already sits where the browser painted it.
  function clippedRect(el) {
    const r = el.getBoundingClientRect();
    let left = r.left, top = r.top, right = r.right, bottom = r.bottom;
    let node = el.parentElement;
    while (node && node !== document.documentElement) {
      const cs = getComputedStyle(node);
      const clipY = cs.overflowY !== 'visible';
      const clipX = cs.overflowX !== 'visible';
      if (clipY || clipX) {
        const cr = node.getBoundingClientRect();
        if (clipY) {
          top = Math.max(top, cr.top);
          bottom = Math.min(bottom, cr.top + node.clientHeight);
        }
        if (clipX) {
          left = Math.max(left, cr.left);
          right = Math.min(right, cr.left + node.clientWidth);
        }
      }
      node = node.parentElement;
    }
    // Finally, nothing paints outside the viewport.
    left = Math.max(left, 0);
    top = Math.max(top, 0);
    right = Math.min(right, window.innerWidth);
    bottom = Math.min(bottom, window.innerHeight);
    const width = right - left;
    const height = bottom - top;
    return { left, top, right, bottom, width, height };
  }

  const textBoxes = Array.from(document.querySelectorAll('*'))
    .filter((el) => {
      if (isVisuallyOccluded(el)) return false;
      if (el.closest('.truncate')) return false;
      const cs = getComputedStyle(el);
      if (cs.position === 'absolute' || cs.position === 'fixed' || cs.position === 'relative')
        return false;
      if (cs.transform !== 'none' || cs.visibility === 'hidden') return false;
      return Array.from(el.childNodes).some(
        (n) => n.nodeType === Node.TEXT_NODE && n.textContent.trim().length > 0
      );
    })
    .map((el) => ({ el, r: clippedRect(el) }))
    .filter((b) => b.r.width > 0 && b.r.height > 0)
    .filter((b) => b.r.width > 0 && b.r.height > 0)

  for (let i = 0; i < textBoxes.length; i++) {
    for (let j = i + 1; j < textBoxes.length; j++) {
      const a = textBoxes[i].r;
      const b = textBoxes[j].r;
      // A child's box is always inside its parent's box, so "X overlaps X's
      // parent" is a tautology rather than a layout defect. Only compare
      // boxes that are geometrically independent of each other.
      const ea = textBoxes[i].el;
      const eb = textBoxes[j].el;
      if (ea.contains(eb) || eb.contains(ea)) continue;
      const ox = Math.min(a.right, b.right) - Math.max(a.left, b.left);
      const oy = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
      if (ox > 2 && oy > 2) {
        // Same-line whitespace collisions are normal; require real overlap.
        const area = Math.min(a.width * a.height, b.width * b.height);
        if (ox * oy > area * 0.35) {
          fails.push({
            check: 'overlap',
            detail: \`"\${ea.textContent.trim().slice(0, 24)}" overlaps "\${eb.textContent.trim().slice(0, 24)}"\`,
            selector: ea.tagName.toLowerCase(),
          });
        }
      }
    }
  }

  // ── Craft checks (from the website-design skill's probe) ──
  //
  // The repo's own probe covers contrast, overflow, headings and overlap.
  // These four catch the defects a dashboard actually ships with: a word
  // split mid-letter by a track that is too narrow, a web font that was
  // never loaded (so the page silently renders in a fallback face), a
  // grid child squeezed to 0px, and a type system that has sprawled past
  // the point where it reads as a system.

  const sel = (e) => {
    if (!e || e.nodeType !== 1) return '?';
    let s = e.tagName.toLowerCase();
    if (e.id) return s + '#' + e.id;
    if (e.classList.length) s += '.' + [...e.classList].slice(0, 2).join('.');
    return s;
  };

  const shown = (e) => {
    if (e.checkVisibility && !e.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })) return false;
    for (let n = e; n && n.nodeType === 1; n = n.parentElement) {
      const s = getComputedStyle(n);
      if (s.display === 'none' || s.visibility === 'hidden' || +s.opacity === 0) return false;
    }
    return true;
  };

  // ── Document basics ──
  if (!document.documentElement.lang) {
    fails.push({ check: 'doc', detail: 'html has no lang attribute', selector: 'html' });
  }
  if (!document.title.trim()) {
    fails.push({ check: 'doc', detail: 'document has no <title>', selector: 'head' });
  }
  if (!document.querySelector('meta[name="viewport"]')) {
    fails.push({ check: 'doc', detail: 'no meta viewport', selector: 'head' });
  }

  // ── Collapsed grid/flex items ──
  // A card with no width is invisible, but the row around it still occupies
  // space, so the page looks like a gap rather than a bug.
  for (const g of document.querySelectorAll('*')) {
    const s = getComputedStyle(g);
    if (!/(grid|flex)/.test(s.display) || !shown(g)) continue;
    for (const c of g.children) {
      if (!shown(c) || !c.textContent.trim() || getComputedStyle(c).position === 'absolute') continue;
      if (c.getBoundingClientRect().width >= 2) continue;
      // A zero-width fill inside a progress track is INTENTIONAL: the bar
      // is at 0%, and the track itself is the visible element. The same is
      // true of a fill whose own width is set inline. Only a genuinely
      // squeezed CONTENT block is a defect, so require the element to be
      // laid out by the parent's track sizing rather than sized by itself.
      const cs = getComputedStyle(c);
      if (cs.width !== 'auto' || cs.inlineSize !== 'auto') continue;
      if (cs.flexBasis && cs.flexBasis !== 'auto' && cs.flexBasis !== '0px') continue;
      fails.push({
        check: 'collapsed-item',
        detail: \`\${sel(c)} ("\${c.textContent.trim().slice(0, 28)}") is squeezed to 0px wide\`,
        selector: sel(c),
      });
    }
  }

  // ── Broken words ──
  // A word of 4+ letters at 18px+ that renders on two lines means the track
  // is too narrow and the word is being split mid-letter.
  const textNodes = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const t = walker.currentNode;
    if (!t.textContent.trim()) continue;
    const el = t.parentElement;
    if (!el || ['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE'].includes(el.tagName)) continue;
    if (!shown(el)) continue;
    textNodes.push({ el, tn: t, t: t.textContent.trim().slice(0, 32) });
  }
  for (const n of textNodes) {
    const fs = parseFloat(getComputedStyle(n.el).fontSize);
    if (fs < 18) continue;
    const re = /\\S+/g;
    let m;
    while ((m = re.exec(n.tn.textContent))) {
      const w = m[0];
      if (w.length < 4 || /[-‐‑/]/.test(w)) continue;
      const r = document.createRange();
      r.setStart(n.tn, m.index);
      r.setEnd(n.tn, m.index + w.length);
      const rects = [...r.getClientRects()].filter((q) => q.width > 1);
      if (rects.length > 1 && Math.abs(rects[0].top - rects[rects.length - 1].top) > 2) {
        fails.push({
          check: 'broken-word',
          detail: \`"\${w}" splits across lines at \${fs}px in \${sel(n.el)} (\${Math.round(n.el.getBoundingClientRect().width)}px wide)\`,
          selector: sel(n.el),
        });
        break;
      }
    }
  }

  // ── Font fallback ──
  // A named face with no @font-face (or a @font-face that never loaded)
  // silently renders in the fallback, so the type voice the design asked for
  // is not the type voice on screen.
  const norm = (f) => f.trim().replace(/^["']|["']$/g, '').toLowerCase();
  // A family is LOADED when AT LEAST ONE of its faces is loaded.
  //
  // A webfont arrives as many faces - one per weight AND per unicode
  // subset (latin, cyrillic, greek). The browser loads a face only when
  // some text needs it, so a family that IS working still has faces
  // sitting at \`unloaded\`. The old check compared the family against
  // the set of LOADED faces, so Inter - whose latin face was loaded and
  // whose cyrillic face was not - read as "never loaded", and the
  // probe reported a font failure on every page.
  const loaded = new Set();
  for (const f of document.fonts) {
    if (f.status === 'loaded') loaded.add(norm(f.family));
  }
  const declared = new Set([...document.fonts].map((f) => norm(f.family)));
  const systemFace =
    /^(ui-|system-ui|-apple-system|blinkmacsystemfont|segoe ui|roboto$|helvetica|arial|georgia|times|menlo|monaco|consolas|courier|sfmono|sans-serif|serif|monospace|cursive|fantasy|math|emoji)/;
  const asked = new Map();
  for (const n of textNodes) {
    const f = norm(getComputedStyle(n.el).fontFamily.split(',')[0]);
    asked.set(f, (asked.get(f) || 0) + 1);
  }
  for (const [f, count] of asked) {
    if (systemFace.test(f) || loaded.has(f)) continue;
    fails.push({
      check: 'font-fallback',
      detail: \`"\${f}" is asked for by \${count} text nodes but \${declared.has(f) ? 'its @font-face never loaded' : 'no @font-face declares it'}\`,
      selector: 'html',
    });
  }

  // ── Type and colour sprawl ──
  // Past a point the type system stops being a system. These are advisory:
  // a dense dashboard legitimately carries more sizes than a landing page.
  const sizes = new Set(textNodes.map((n) => Math.round(parseFloat(getComputedStyle(n.el).fontSize))));
  if (sizes.size > 16) {
    warns.push({
      check: 'type-sprawl',
      detail: \`\${sizes.size} distinct font sizes (\${[...sizes].sort((a, b) => a - b).join(', ')})\`,
    });
  }
  // INLINE-STYLED colours are EXCLUDED from the count, and this is the
  // difference between an advisory that means something and one that does
  // not. A tag or campaign chip paints an inline style computing its ink
  // from the operator's chosen hex, against that chip's own fill,
  // precisely because the hue is the operator's choice and its luminance is
  // not the app's to control (frontend/src/lib/chip-colors.ts argues this
  // at length, and correctly).
  //
  // Counting those against the app's palette reported 23 distinct text
  // colours on the composer - 20 of them one swatch each, from six tags.
  // The advisory was measuring the operator's colour input, not the type
  // system, so it could never name a real defect.
  //
  // The metric now counts the app's OWN text colours - what a stylesheet
  // assigns. The composer is four of them (content, muted, faint, error)
  // plus the accent-fg on its one filled button. That IS a system. A page
  // whose source genuinely assigns more than 16 is not, and is still
  // reported.
  //
  // The one inline colour that SHOULD count - a hardcoded literal in
  // source - is caught by check-tailwind-classes.mjs and the hex gate in
  // make check, which read the SOURCE rather than the rendered styles, so
  // it cannot escape here.
  //
  // An inline colour counts only when the element sets it on ITSELF, so an
  // inherited chip colour does not wrongly exempt the child's text.
  //
  // The inline literal that SHOULD count is caught by
  // check-tailwind-classes.mjs, which reads the SOURCE, so this exemption
  // is not a hole.
  const colors = new Set(
    textNodes
      .filter((n) => !n.el.style || !n.el.style.color)
      .map((n) => getComputedStyle(n.el).color)
  );
  if (colors.size > 16) {
    warns.push({
      check: 'color-sprawl',
      detail: \`\${colors.size} distinct text colors\`,
    });
  }

  // ── Tap targets (advisory) ──
  //
  // WCAG 2.2's target-size rule is about the CLICKABLE AREA, not the
  // control's own box. A checkbox inside its own <label> is clicked
  // through the label, so the label is the target - measuring the 16px
  // input instead reports a violation that does not exist. Same for an
  // <input> inside a <button> or a <label>.
  const targetRect = (el) => {
    const own = el.getBoundingClientRect();
    if (own.height >= 44 && own.width >= 44) return own;
    if (el.tagName === 'INPUT') {
      const wrapper = el.closest('label, button, [role="button"]');
      if (wrapper && wrapper !== el) {
        const r = wrapper.getBoundingClientRect();
        if (r.width > 0 && r.height > 0) return r;
      }
    }
    return own;
  };
  for (const el of document.querySelectorAll('button,a[href],[role="button"],input,select')) {
    // A HIDDEN control is not a tap target. The composer uses a
    // \`hidden\` file input for its uploads, and it measured 199x20 -
    // a violation that does not exist, because nothing can click it.
    // \`offsetParent\` is null for a \`display: none\` element, and it is
    // also null for \`position: fixed\`, so the check is on the computed
    // visibility and the rendered box instead.
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    const r = targetRect(el);
    if (r.width === 0 || r.height === 0) continue;
    // A VISUALLY HIDDEN control is not a tap target either.
    //
    // The skip link is the case: it is clipped to 1x1 until it takes the
    // focus, which is the correct pattern - a keyboard user Tab's to it
    // and it becomes visible. Measured in its resting state it is a
    // 1x1 target, and reporting that as a violation is reporting a
    // violation of the PATTERN.
    //
    // The \`sr-only\` pattern is exactly this: \`clip: rect(0,0,0,0)\` with
    // a 1x1 box, or the text-indent trick. Both render nothing, so
    // nothing can be aimed at them.
    //
    // The test is the COMBINATION: a box of 1x1 that is clipped away
    // (or pushed off-screen by a huge negative text-indent). A 1x1 box
    // on its own is a real control that happens to be tiny; a 1x1 box
    // that is clipped to nothing is the \`sr-only\` pattern, and nothing
    // can be aimed at it.
    const clippedAway =
      (cs.clip && cs.clip !== 'auto' && /rect\\(\\s*0(px)?\\s*,\\s*0(px)?\\s*,\\s*0(px)?\\s*,\\s*0(px)?\\s*\\)/.test(cs.clip)) ||
      (cs.clipPath && cs.clipPath !== 'none') ||
      parseFloat(cs.textIndent) < -100;
    if (clippedAway && r.width <= 2 && r.height <= 2) continue;
    // WCAG 2.2 "Target Size (Minimum)" (2.5.8) sets the floor at 24x24
    // CSS pixels. Below that the control is a FAIL - a real
    // accessibility failure.
    //
    // 44x44 is the AAA-ish target the skill's own rules ask for, and
    // the size most touch interfaces aim at. A target between the two
    // is a WARNING, not a failure: the month grid's chips are 144x24,
    // which is a legitimate dense-overview cell and clears the
    // standard's floor.
    //
    // Before this split, every dense-grid control was a FAIL, and a
    // FAIL means "the probe is not clean" - so a page with a
    // legitimate month view could never pass. The severity has to
    // match the standard, or the gate lies.
    if (r.height < 24 || r.width < 24) {
      fails.push({
        check: 'tap-target',
        detail: \`\${Math.round(r.width)}x\${Math.round(r.height)} < 24x24 — "\${(el.textContent || el.getAttribute('aria-label') || el.tagName).trim().slice(0, 30)}"\`,
        cls: String(el.className || '').slice(0, 160),
        tag: el.tagName,
      });
    } else if (r.height < 44 || r.width < 44) {
      warns.push({
        check: 'tap-target',
        detail: \`\${Math.round(r.width)}x\${Math.round(r.height)} < 44x44 — "\${(el.textContent || el.getAttribute('aria-label') || el.tagName).trim().slice(0, 30)}"\`,
        cls: String(el.className || '').slice(0, 160),
        tag: el.tagName,
      });
    }
  }

  // ── Tiny text (advisory) ──
  for (const el of document.querySelectorAll('*')) {
    const cs = getComputedStyle(el);
    const size = parseFloat(cs.fontSize);
    if (size > 0 && size < 10) {
      const own = Array.from(el.childNodes).some(
        (n) => n.nodeType === Node.TEXT_NODE && n.textContent.trim().length > 0
      );
      if (own) {
        warns.push({
          check: 'tiny-text',
          detail: \`\${size}px — "\${el.textContent.trim().slice(0, 30)}"\`,
        });
      }
    }
  }

  return { fails, warns };
`;

// AUDIT_BODY already has the core spliced in at its ${CONTRAST_CORE} marker -
// the template interpolates at module load, so no second copy here.
const AUDIT = new Function(AUDIT_BODY);

// ── Runner ───────────────────────────────────────────────────

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function login(page) {
  if (!PASSWORD) {
    throw new Error('No password: pass --password or set APP_PASSWORD');
  }
  // The CSRF middleware requires an Origin matching FRONTEND_URL on
  // state-changing requests, and a bare request-context POST sends none.
  const res = await page.request.post(`${BASE_URL}/api/auth/login`, {
    data: { password: PASSWORD },
    headers: { Origin: BASE_URL },
  });
  if (!res.ok()) {
    throw new Error(`Login failed: HTTP ${res.status()}`);
  }

  // ── SET THE ONBOARDED FLAG, at login, once ──
  //
  // MEASURED 2026-10-31: the per-route `dismissOnboarding()` below could
  // not dismiss anything. It clicks the tour's Skip control, but the tour
  // is NOT up in the moment right after the login POST (the layout only
  // raises it once the app is authenticated AND a route has mounted), so
  // `skip.isVisible()` was false, nothing was clicked, and the tour then
  // covered every route measured after it - /calendar, /channels and /
  // all opened with `Welcome to Switchpost` up and
  // `localStorage['switchpost-onboarded']` still null.
  //
  // The flag is what the layout checks on every mount, so setting it once
  // here is what actually stops the tour returning on the next
  // navigation. Clicking Skip alone does not persist.
  //
  // IT MUST BE SET ON THE ORIGIN, and that is not where this function
  // runs. The probe logs in with `page.request.post`, which never
  // navigates, so the page is still at `about:blank` - and on
  // `about:blank` reading `localStorage` throws
  // `SecurityError: Access is denied for this document`. Verified: the
  // flag stayed null and the tour was still up on the next route. A bare
  // try/catch around the write hides that completely, so the write is
  // preceded by a real navigation to the app's origin, where the storage
  // partition is the one the routes will use.
  await page.goto(`${BASE_URL}/login`, { waitUntil: 'domcontentloaded' });
  await page.evaluate(() => {
    try { localStorage.setItem('switchpost-onboarded', 'true'); } catch {}
  });
}

/// Dismiss the first-run onboarding overlay if it is up.
///
/// On an empty database the modal covers every route, so measuring the page
/// underneath it measures the modal, not the page. The dismiss is a click on
/// its own Skip control — the documented way out — and it is skipped when no
/// overlay is present (a seeded database never shows one).
async function dismissOnboarding(page) {
  try {
    const skip = page.getByRole('button', { name: /skip|quick tour|got it/i }).first();
    if (await skip.isVisible({ timeout: 1500 })) {
      await skip.click({ timeout: 2000 });
      await sleep(400);
    }
  } catch {
    // No overlay, or it closed on its own — nothing to dismiss.
  }
}

// Click a control by its visible label, so a page can be measured in a
// state other than its default.
//
// The calendar has THREE views - week, month and day - and each is its
// own component with its own layout. The 2026-10-20 red-team found the
// month view's chips squeezed to 15px of a 144px cell, and no amount of
// probing the default (week) view would ever have seen it.
//
// So a route can be probed in a named state: `--state "Month"`.
const STATE_CONTROLS = flag('state', '').split(',').filter(Boolean);

async function applyStates(page) {
  for (const label of STATE_CONTROLS) {
    const want = label.trim();
    try {
      // The control's accessible name may include an ICON ("📅 Month"),
      // so an exact match on the whole name misses it. Match the label
      // as a WORD, which is what the user sees.
      const btn = page.getByRole('button', { name: new RegExp(`\\b${want}\\b`) }).first();
      if (await btn.isVisible({ timeout: 1500 })) {
        await btn.click({ timeout: 2000 });
        await sleep(800);
      }
    } catch {
      // The control is not on this page. That is not a finding: the
      // state is an option, not a requirement.
    }
  }
}

async function main() {
  const executablePath = findChrome();
  fs.mkdirSync(OUT_DIR, { recursive: true });

  const browser = await chromium.launch({
    executablePath,
    ignoreHTTPSErrors: true,
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });

  const summary = { fails: [], warns: [], skipped: [], visits: 0, errors: [], notFound: [], providerErrors: [] };

  // The route list is resolved ONCE, before the theme loop, because the
  // dynamic routes need a session and the ids are the same in both themes.
  // Resolving per theme would double the API calls and could pick two
  // different ids if the data changed between them.
  let routes = [...STATIC_ROUTES];
  if (!LOGGED_OUT) {
    const probeCtx = await browser.newContext({ ignoreHTTPSErrors: true });
    const probePage = await probeCtx.newPage();
    await login(probePage);
    for (const spec of RESOLVED_ROUTES) {
      const res = await probePage.request.get(`${BASE_URL}${spec.from}`);
      let id = null;
      try {
        const body = await res.json();
        id = spec.pick(body);
      } catch {
        id = null;
      }
      if (id) {
        routes.push(spec.path.replace('{id}', id));
      } else if (res.status() !== 200) {
        // The API itself failed. That IS a finding.
        summary.fails.push({
          route: spec.path,
          width: '',
          theme: '',
          check: 'unreachable-route',
          detail: `no id from ${spec.from} (HTTP ${res.status()}) - the page cannot be reached`,
        });
      } else {
        // The API answered 200 with an EMPTY list, so the instance has
        // no row to open. The page is not broken - there is nothing to
        // open. A fresh install looks exactly like this, and a fresh
        // install is the FIRST thing a founder sees.
        //
        // This was a FAIL, which meant the probe could never be clean
        // on a new instance, and "clean on a new instance" is the one
        // state that matters most.
        summary.skipped.push({
          route: spec.path,
          check: 'unreachable-route',
          detail: `no row to open - ${spec.from} returned an empty list`,
        });
      }
    }
    await probeCtx.close();
  }
  const ROUTES = routes;

  for (const theme of THEMES) {
    const context = await browser.newContext({
      viewport: { width: 1440, height: 900 },
      ignoreHTTPSErrors: true,
      deviceScaleFactor: 1,
    });
    // Seed the theme through the app's own store channel: localStorage is what
    // `theme.init()` reads on mount. Toggling the documentElement class directly
    // is untracked and reverted by init(), which leaves the page with no theme
    // class at all — the tokens then resolve to nothing and every contrast
    // reading is garbage. An init script runs before any page script, so the
    // store sees the value from its very first read.
    await context.addInitScript((t) => {
      try {
        localStorage.setItem('switchpost-theme', t);
      } catch {
        // Storage denied (opaque origin) — the page falls back to dark.
      }
    }, theme);
    const page = await context.newPage();

    page.on('pageerror', (err) => {
      // The STACK is the finding. "i.find is not a function" names the
      // symptom; the stack names the minified frame, which points back
      // at the source. Without it, a pageerror is a dead end.
      const stack = err && err.stack ? String(err.stack) : '';
      const frames = stack
        .split('\n')
        .slice(1, 4)
        .map((l) => l.trim())
        .filter(Boolean)
        .join(' | ');
      summary.fails.push({
        route: '(page lifecycle)',
        width: 0,
        theme,
        check: 'pageerror',
        detail: `${String(err.message || err).slice(0, 200)}${frames ? ` @ ${frames.slice(0, 400)}` : ''}`,
      });
    });

    page.on('console', (msg) => {
      if (msg.type() === 'error') {
        // A missing /api/signatures/default on /posts/new is the
        // documented "no default signature" 404, not an app error.
        const t = msg.text();
        if (t.includes('signatures/default')) return;
        // The LOGGED-OUT pass measures /login by design, and the
        // layout's documented session probe (GET /api/auth/me,
        // +layout.ts) answers 401 there on every visit. Chrome logs
        // that as a console error; it is the expected answer, not a
        // defect. Without this exemption the login page's console
        // reports "8 errors ... (7 more)" on a clean build, and a
        // REAL error would hide inside the noise. Only the logged-out
        // pass is exempted: an authenticated session that trips 401
        // is a defect and stays reported.
        //
        // The message TEXT does not name the URL ("Failed to load
        // resource: ... 401 ()"), so the check reads the source
        // location from `msg.location()` first and only falls back to
        // the text (which a fetch() rejection can carry).
        const srcUrl = (msg.location() && msg.location().url) || '';
        if (
          LOGGED_OUT &&
          /\b401\b/.test(t) &&
          (srcUrl.includes('/api/auth/me') || t.includes('auth/me'))
        ) {
          return;
        }
        summary.errors.push({ theme, detail: t.slice(0, 200) });
      }
    });

    page.on('response', async (res) => {
      if (res.status() >= 500) {
        // A 5xx whose body says `code: "provider"` is the PROVIDER's
        // failure, not this app's.
        //
        // The app labels its own failures: a provider that rejects the
        // request comes back as a 502 with `{"code":"provider"}` and
        // a message naming the provider's reason. A founder with a
        // revoked X cookie sees exactly that, and the page shows the
        // message and a Retry button.
        //
        // Counting it as the app's 5xx is wrong in both directions: it
        // puts a failure the founder caused into the app's defect list,
        // and it means the gate can never be clean on any instance
        // whose credentials have expired. The gate has to measure THIS
        // app, so a provider's failure is reported separately.
        let code = null;
        try {
          const body = await res.json().catch(() => null);
          if (body && typeof body.code === 'string') code = body.code;
        } catch {
          // A body that is not json is not a labelled failure.
        }
        if (code === 'provider') {
          summary.providerErrors.push({
            theme,
            detail: `provider ${res.status()} ${res.url()}`,
          });
          return;
        }
        summary.fails.push({
          route: '(response)',
          width: 0,
          theme,
          check: 'http-5xx',
          detail: `HTTP ${res.status()} ${res.url()}`,
        });
      } else if (res.status() === 404) {
        // A 404 is a defect the visitor can trigger: the page asked for
        // something the server does not serve. Record the URL, not just
        // the count, so the report names the broken call.
        const url = res.url();
        // The favicon is not part of the app surface.
        if (url.includes('/favicon')) return;
        summary.notFound.push({ theme, url });
      } else if (res.status() === 401) {
        // A 401 is tracked BY URL because the console text does not
        // name the source ("Failed to load resource: ... 401 ()"),
        // so a genuinely unauthorized call was indistinguishable
        // from the layout's documented session probe in the console
        // summary. Here the two CAN be told apart:
        //   - /api/auth/me on the LOGGED-OUT pass: the expected
        //     answer of the auth guard (+layout.ts) - exempted.
        //   - anything else, or any 401 on an authenticated pass: a
        //     defect (an expired session bouncing a page's data call,
        //     or a route the guard failed to protect) - report it.
        const url = res.url();
        if (LOGGED_OUT && url.includes('/api/auth/me')) return;
        summary.fails.push({
          route: '(response)',
          width: 0,
          theme,
          check: 'http-401',
          detail: `HTTP 401 ${url}`,
        });
      }
    });

    if (!LOGGED_OUT) {
      await login(page);
    }

    for (const route of ROUTES) {
      for (const width of WIDTHS) {
        await page.setViewportSize({ width, height: 900 });
        const url = `${BASE_URL}${route}`;
        try {
          await page.goto(url, { waitUntil: 'load', timeout: 30_000 });
          // Wait for the APP to render, not for a fixed delay. The `load`
          // event fires before SvelteKit's client boot has run the
          // layout's auth probe, so a fixed `sleep(700)` measured the
          // page mid-boot and reported a blank document as "no <h1> on
          // the page" - on whichever route happened to be slow. The
          // render time ranged from 115ms to 2200ms across the routes,
          // so no fixed sleep is safe.
          await page
            .waitForSelector('h1', { state: 'attached', timeout: 10_000 })
            .catch(() => {
              // A route with genuinely no heading is a real finding, and
              // the audit below reports it. Waiting for the timeout is
              // the slow path but it keeps the report honest.
            });
          await sleep(700); // let the first data fetch settle
          // The onboarding modal covers every route on an empty database;
          // measuring it would measure the modal, not the page.
          await dismissOnboarding(page);
          await applyStates(page);

          // Wait for the webfont before auditing.
          //
          // A webfont arrives as many faces, and the browser loads a
          // face only when some text needs it. The font-fallback check
          // reads `document.fonts` and asks whether the family is
          // LOADED - so auditing before the font settled reported
          // "inter never loaded" on a page where Inter was about to be
          // used. It was a race, not a defect, and it made the probe's
          // font result a coin toss.
          await page.evaluate(() => document.fonts?.ready).catch(() => {});

          // The ENTRY ANIMATION must have finished before the audit
          // measures anything.
          //
          // Every page fades in over 0.2s (`.page-enter`), and the
          // fade is an `opacity` animation on an ANCESTOR of the text.
          // The contrast check treats an ancestor with `opacity < 1` as
          // a translucent group and stops the backdrop walk there - so
          // a page measured mid-fade was measured against the wrong
          // background.
          //
          // It showed up on /tags at 375px, where the taller page took
          // longer to settle: the tag names measured 1.17:1 and the
          // edit and delete buttons 2.37:1, and both were FINE - the
          // real readings are 15.25:1 and 7.21:1. A false FAIL, on a
          // page that was correct.
          //
          // So the probe waits for the running animations to finish.
          // A page that ANIMATES forever (the skeleton shimmer) would
          // hang, so only the ones that are not infinite are awaited.
          await page
            .evaluate(async () => {
              const anims = document.getAnimations().filter((a) => {
                const t = a.effect?.getComputedTiming?.();
                return t && t.iterations !== Infinity;
              });
              if (!anims.length) return;
              await Promise.all(
                anims.map((a) => a.finished.catch(() => undefined))
              );
            })
            .catch(() => {});

          // The theme must be APPLIED before the audit measures
          // anything.
          //
          // The theme is a class on `documentElement`, applied by the
          // theme store on mount. The probe seeds the choice through
          // localStorage, and the store reads it on its first read - so
          // there is a window in which the page is rendered with NO
          // theme class, and the tokens resolve to the un-themed
          // defaults.
          //
          // A contrast audit run in that window measures the wrong
          // colours: one run reported 69 failures on the research
          // page, every one of them a text tier against a background
          // it would never meet, and the next run of the same command
          // reported none.
          //
          // A measurement that depends on WHEN it ran is not a
          // measurement. So the audit waits for the class.
          await page
            .waitForFunction(
              (t) =>
                document.documentElement.classList.contains(t) ||
                document.documentElement.classList.contains(t === 'dark' ? 'light' : 'dark'),
              theme,
              { timeout: 5_000 },
            )
            .catch(() => {
              // A page that never gets a theme class is itself a
              // finding; the audit below measures it as it is.
            });

          const { fails, warns } = await page.evaluate(AUDIT);
          for (const f of fails) {
            summary.fails.push({ route, width, theme, ...f });
          }
          for (const w of warns) {
            summary.warns.push({ route, width, theme, ...w });
          }
          summary.visits += 1;

          const stateSuffix = STATE_CONTROLS.length
            ? `_${STATE_CONTROLS.map((s) => s.trim().replace(/[^a-z0-9]+/gi, '-')).join('-')}`
            : '';
          const name = `${theme}-${width}-${route.replace(/[/:]/g, '_') || 'root'}${stateSuffix}`;
          // The app shell's <main> is the scroll container, so the
          // DOCUMENT never scrolls and `fullPage: true` captures only
          // the viewport - the research page's timing table, the
          // campaign detail's Posts tab and the settings forms all
          // live below the fold and would never be seen.
          //
          // So the screenshot is taken with the viewport TALL enough
          // to hold the tallest scroll container on the page. A
          // screenshot that cannot see the section under review is
          // not a visual verification of it.
          const inner = await page.evaluate(() => {
            let tallest = 0;
            document.querySelectorAll('*').forEach((el) => {
              const oy = getComputedStyle(el).overflowY;
              if ((oy === 'auto' || oy === 'scroll') && el.scrollHeight > tallest) {
                tallest = el.scrollHeight;
              }
            });
            return tallest;
          });
          const shotHeight = Math.min(Math.max(inner + 40, 900), 8000);
          if (shotHeight > 900) {
            await page.setViewportSize({ width, height: shotHeight });
            await page.waitForTimeout(400);
          }
          await page.screenshot({
            path: path.join(OUT_DIR, `${name}.png`),
            fullPage: false,
          });
          if (shotHeight > 900) {
            await page.setViewportSize({ width, height: 900 });
            await page.waitForTimeout(200);
          }
        } catch (e) {
          summary.fails.push({
            route,
            width,
            theme,
            check: 'navigation',
            detail: String(e.message || e).slice(0, 200),
          });
        }
      }
    }

    await context.close();
  }

  await browser.close();

  // ── Report ──
  const lines = [];
  lines.push(`routes: ${ROUTES.length} · widths: ${WIDTHS.join(', ')} · themes: ${THEMES.join(', ')}`);
  lines.push(`visits: ${summary.visits}`);
  lines.push('');
  lines.push(`FAIL: ${summary.fails.length}`);
  for (const f of summary.fails) {
    const at = [f.route, f.width ? `${f.width}px` : '', f.theme].filter(Boolean).join(' ');
    lines.push(`  ✗ [${f.check}] ${at} — ${f.detail}`);
  }
  if (summary.skipped.length) {
    lines.push('');
    lines.push(`SKIPPED: ${summary.skipped.length}`);
    for (const s of summary.skipped) {
      lines.push(`  - [${s.check}] ${s.route} — ${s.detail}`);
    }
  }
  if (summary.providerErrors.length) {
    lines.push('');
    lines.push(`PROVIDER ERRORS: ${summary.providerErrors.length}`);
    lines.push('  (the provider rejected the request - not this app)');
    for (const s of summary.providerErrors) {
      lines.push(`  - ${s.theme} — ${s.detail}`);
    }
  }
  if (summary.errors.length) {
    lines.push('');
    lines.push(`console errors: ${summary.errors.length}`);
    for (const e of summary.errors.slice(0, 20)) {
      lines.push(`  ! ${e.detail}`);
    }
  }
  if (summary.notFound.length) {
    lines.push('');
    lines.push(`404 responses: ${summary.notFound.length}`);
    const seen = new Map();
    for (const n of summary.notFound) {
      const k = n.url;
      seen.set(k, (seen.get(k) || 0) + 1);
    }
    for (const [url, n] of seen) {
      lines.push(`  ? ${n}x ${url}`);
    }
  }
  lines.push('');
  lines.push(`WARN: ${summary.warns.length}`);
  const byCheck = {};
  for (const w of summary.warns) {
    (byCheck[w.check] ||= []).push(w);
  }
    for (const [check, items] of Object.entries(byCheck)) {
      lines.push(`  · ${check}: ${items.length}`);
      // Break the advisory down by width, so "3002 warnings" becomes
      // "these are all the 375px drawer" or "these are everywhere".
      const byWidth = {};
      for (const w of items) {
        const k = `${w.width}px`;
        byWidth[k] = (byWidth[k] || 0) + 1;
      }
      lines.push(
        `      by width: ${Object.entries(byWidth)
          .map(([k, v]) => `${k}=${v}`)
          .join(' ')}`
      );
      // Group by the offending element so the same control is not counted
      // 78 times as 78 separate findings.
      const byControl = new Map();
      for (const w of items) {
        const label = (w.detail.match(/— "(.*)"/) || [0, "?"])[1];
        const key = `${label}`;
        byControl.set(key, (byControl.get(key) || 0) + 1);
      }
      const ranked = [...byControl.entries()].sort((a, b) => b[1] - a[1]);
      lines.push(`      by control (top 15 of ${ranked.length}):`);
      for (const [label, n] of ranked.slice(0, 15)) {
        lines.push(`        ${n}x ${label}`);
      }
      for (const w of items.slice(0, 6)) {
        lines.push(`      ${w.route} @${w.width}px — ${w.detail}`);
      }
    }

  const report = lines.join('\n');
  fs.writeFileSync(path.join(OUT_DIR, 'report.txt'), report);
  console.log(report);

  // The machine-readable form. The text report groups and TRUNCATES, so
  // it cannot answer "which element, on which route, at which width" -
  // and that is the only question a fix needs answered. Every warning
  // carries the route, the width, the theme and the element's own
  // class, so the fix can be located without re-running the browser.
  fs.writeFileSync(
    path.join(OUT_DIR, 'report.json'),
    JSON.stringify(
      { fails: summary.fails, warns: summary.warns, skipped: summary.skipped },
      null,
      2
    )
  );

  process.exit(summary.fails.length === 0 ? 0 : 1);
}

main().catch((e) => {
  console.error(`probe: ${e.message}`);
  process.exit(2);
});
