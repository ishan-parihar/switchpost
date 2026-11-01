#!/usr/bin/env node
// The "ink on a fill" gate.
//
// The point: a solid semantic fill (bg-error, bg-success, bg-warning, bg-info)
// is a PAIRED rule, not a single class. The FILL picks a hue; the INK has to be
// the one that clears the AA floor ON THAT HUE. Those are two different colors,
// and the app's own tokens say so: --error-rgb is red-500 in the dark theme
// (255 100 100), and --error-fg-rgb is the DARK SURFACE ink (11 14 20), because
// white on red-500 is 3.61:1 and the floor for small text is 4.5:1.
//
// So the pair is `bg-error text-error-fg`, exactly as `bg-accent-fill
// text-accent-fg` is. A site that writes `bg-error text-accent-fg` -- or that
// writes `bg-error` with no ink class at all -- is not a typo that shows up in
// a build error, a type error or a screenshot. The button renders, it reads as
// a design choice, and the founder on the dark theme cannot read the label.
//
// That is what the 2026-11-01 audit found: the shared areYouSure() danger
// confirm had `text-accent-fg` (white) on `bg-error`, which measured 2.89:1 on
// the dark theme. Every destructive confirm in the app goes through that one
// button -- 35 call sites -- so the defect was in all of them at once. Two
// sibling sites had the same shape and one had no ink class at all.
//
// This script makes that a build failure.
//
// THE TRAP the first version of this gate fell into, recorded here so it is not
// built again: it skipped ANY class attribute containing `?`, `&&`, `|` or `+`,
// on the grounds that a "dynamic" class list cannot be judged from the source.
// The one site the audit had just fixed was exactly such a list --
// class="{cond ? 'text-error-fg bg-error' : 'text-accent-fg bg-accent-fill'}" --
// so the gate exempted the very defect it was written to catch and reported
// 2713 pairs "correct" with the bug restored. A gate that measures nothing is
// worse than no gate, which is the whole lesson of the audit series this repo
// keeps re-learning.
//
// A conditional is not opaque. It has BRANCHES, each an independent class list,
// and each is judgeable on its own. So this gate splits a class list on
// ternaries and `&&`, reads the CLASS TOKENS in each branch (ignoring the
// expression around them), and applies the rule to every branch. The mutation
// is below and it does bite.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, extname } from 'node:path';

const SRC = new URL('../src', import.meta.url).pathname;

// The semantic fills that have a dedicated `*-fg` ink token. The pairing rule
// is per fill, so a list can be judged one fill at a time.
const FILLS = ['error', 'success', 'warning', 'info'];

const CLASS_ATTR = /\sclass=\{?"((?:[^"\\]|\\.)*)"/g;

/** Walks src/ and yields .svelte file paths. */
function* svelteFiles(dir) {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    const st = statSync(p);
    if (st.isDirectory()) yield* svelteFiles(p);
    else if (extname(p) === '.svelte') yield p;
  }
}

/**
 * Splits one class attribute value into INDEPENDENT class lists, one per branch
 * of every ternary and every `&&`. The expression itself is dropped, but the
 * class tokens inside each branch survive, because `cond ? 'a b' : 'c d'` is
 * about which of {a,b} or {c,d} applies, never about inventing new tokens.
 *
 * `<div class="x {c ? 'bg-error text-error-fg' : 'bg-accent-fill text-accent-fg'} y">`
 * becomes ['x  y', 'bg-error text-error-fg', 'bg-accent-fill text-accent-fg'].
 */
function classBranches(raw) {
  return raw
    .split(/\?|&&|\|\|/)
    .map(part => part
      // Drop the Svelte expression's own syntax: quotes, braces, commas, calls.
      .replace(/['"`{}()[\]]/g, ' ')
      .replace(/\s+/g, ' ')
      .trim())
    .filter(part => part.length > 0);
}

/** True when this branch's class list has a token with the given prefix. */
function hasToken(branch, prefix) {
  return new RegExp(`(^|\\s)${prefix}(\\s|$)`).test(branch);
}

const violations = [];
let pairsChecked = 0;
let elementsChecked = 0;

for (const file of svelteFiles(SRC)) {
  const src = readFileSync(file, 'utf8');
  const lines = src.split('\n');
  let m;
  CLASS_ATTR.lastIndex = 0;
  while ((m = CLASS_ATTR.exec(src)) !== null) {
    const branches = classBranches(m[1]);
    // The element's text, for the "is there anything to read?" question: read
    // forward from the class attribute to the tag's `>`.
    // The element's TEXT is what the rule is about. An element that closes
    // empty (`<span class="...bg-error"></span>`) is a dot or a bar: nothing
    // to read, no rule to break, per WCAG's own exemption.
    //
    // The read has to be exact about where the TAG ends, and it went wrong
    // twice before this note existed:
    //   1. `src.indexOf('>', m.index)` found a `>` AFTER the tag, so the slice
    //      had no tag terminator and `>`-anchored regexes never matched.
    //   2. A fallback `/[A-Za-z{]/` then treated the INTERPOLATED ATTRIBUTES
    //      (`style="width: {pct}"`) as element text, so an empty dot with a
    //      bound attribute was flagged.
    // So: find the real `>` after the attribute (attributes cannot contain a
    // bare `>` outside a quote), then look at what follows BEFORE the matching
    // close, and count only a letter or an interpolation `{`.
    // Walk FORWARD from the class attribute over the remaining attributes of
    // this same tag, so a `>` inside an interpolated attribute value or inside
    // a following `style="{cond ? ... : ...}"` does not end the tag early.
    // A tag ends at a `>` that is not inside a quoted attribute value.
    let i = m.index + m[0].length;
    let quote = null;
    for (; i < src.length; i++) {
      const c = src[i];
      if (quote) { if (c === quote) quote = null; continue; }
      if (c === '"' || c === "'") { quote = c; continue; }
      if (c === '>') break;
    }
    const tagEnd = i;
    const attrRegion = src.slice(m.index, tagEnd);
    const bodyStart = tagEnd + 1;
    const body = src.slice(bodyStart, bodyStart + 200);
    const isSelfClosing = src[tagEnd - 1] === '/';
    const closesEmpty = /^\s*<\//.test(body);
    const isDecoration = /aria-hidden="true"/.test(attrRegion);
    // A letter or an interpolation that yields text: tags stripped first, so
    // the element's own child tags (icons, nested controls) do not count.
    const hasText = !isSelfClosing && !closesEmpty && /[A-Za-z{]/.test(body.replace(/<[^>]*>/g, ' '));
    elementsChecked++;

    for (const branch of branches) {
      for (const fill of FILLS) {
        // SOLID fill only. `bg-error/10` is an alpha WASH: its ink is the normal
        // content tier, already covered by the browser contrast gates.
        if (!hasToken(branch, `bg-${fill}`) || hasToken(branch, `bg-${fill}/`)) continue;
        pairsChecked++;
        if (hasToken(branch, `text-${fill}-fg`)) continue;
        // A fill with the wrong or missing ink. Only a defect if there is text
        // to read -- a dot or an empty badge breaks no contrast rule.
        if (!hasText || isDecoration) continue;
        const lineNo = src.slice(0, m.index).split('\n').length;
        violations.push({
          file: file.replace(SRC, 'src'),
          line: lineNo,
          fill,
          branch,
          near: lines[lineNo - 1].trim().slice(0, 110),
        });
      }
    }
  }
}

if (violations.length) {
  console.error(`✗ check:ink-on-fill — ${violations.length} solid fill branch(es) with the wrong or missing ink:\n`);
  for (const v of violations) {
    console.error(`  ${v.file}:${v.line}`);
    console.error(`    fill: bg-${v.fill}   needs: text-${v.fill}-fg`);
    console.error(`    branch: ${v.branch}`);
    console.error(`    source: ${v.near}\n`);
  }
  console.error('  The rule: a solid semantic fill takes its matching ink — bg-error → text-error-fg,');
  console.error('  bg-accent-fill → text-accent-fg. Dark-theme --error-rgb is LIGHT red (255 100 100)');
  console.error('  and needs DARK ink; white on it measures 3.61:1 against a 4.5:1 floor.');
  process.exit(1);
}

console.log(`✓ check:ink-on-fill — ${pairsChecked} solid fill/ink pair(s) correct across ${elementsChecked} class attribute(s)`);
