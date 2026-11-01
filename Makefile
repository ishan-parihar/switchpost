# ─── Switchpost Makefile ─────────────────────────────────────
# Development targets for fast iteration.
#
# Quick redeploy after code changes:
#   make redeploy
#
# That's it. One command builds + copies + restarts.

APP_NAME    = switchpost
APP_DIR     = $(shell pwd)
BINARY_PATH = /usr/local/bin/$(APP_NAME)
SKILL_SRC   = $(APP_DIR)/skills/switchpost-agent
SKILL_DEST  = $(HOME)/.agents/skills/switchpost-agent

# zigbuild target — glibc 2.36 matches Debian 12 (bookworm) VPS
TARGET      = x86_64-unknown-linux-gnu.2.36
RELEASE_DIR = target/x86_64-unknown-linux-gnu/release

.PHONY: build frontend vendor-pins check audit deploy redeploy restart status logs watch

# ── Build ───────────────────────────────────────────────────────

build: vendor-pins frontend
	cargo zigbuild --release --target $(TARGET)

frontend:
	cd frontend && pnpm install && pnpm build

# ── Vendor pins (plan v25 §9, phase B0) ─────────────────────────
# Verifies every vendored/pinned upstream source against the contract in
# docs/planning/VENDOR_PINS.md. Local drift is fatal (non-zero + diff).
# Upstream advisories (newer release, yanked pin) are reported loudly but do not
# block a local build; VENDOR_PULL_STRICT=1 makes them fatal too. CI uses the
# same gate — see VENDOR_PINS.md §1 for the ratchet to turn strict on for good.
# Run `scripts/vendor-pull.sh --update` after an intentional pin change.
vendor-pins:
	@./scripts/vendor-pull.sh --check

# Fast compile gate (AGENTS.md §3.2). SQLX_OFFLINE is required: the .sqlx/
# cache is committed, so the sqlx macros must not reach for a live DATABASE_URL.
#
# The two SOURCE gates run here, because both need the built frontend:
#
#   check-hex-tokens.mjs       a literal hex outside the token layer does
#                               not retheme, and cannot
#   check-tailwind-classes.mjs a Tailwind class the stylesheet does not
#                               define is a SILENT failure - the element
#                               simply does not get the style, and nothing
#                               reports it. A truncated class name
#                               (`hover:text-accent-stro`) is not a build
#                               error, not a type error and not a
#                               svelte-check warning.
check: vendor-pins
	SQLX_OFFLINE=true cargo check --lib --bin switchpost
	cd frontend && node scripts/check-hex-tokens.mjs
	cd frontend && node scripts/check-tailwind-classes.mjs
	# The ink-on-fill gate: a solid semantic fill takes its MATCHING ink.
	# bg-error needs text-error-fg, never text-error (the same --error-rgb on
	# both sides, ratio 1.00:1) and never text-accent-fg (white on dark-theme
	# red-500, 3.61:1). The 2026-11-01 audit measured both defects: the shared
	# areYouSure() danger confirm at 2.89:1 across 35 call sites, and every
	# error and warning toast at 1.00:1 in both themes.
	cd frontend && node scripts/check-ink-on-fill.mjs
	python3 scripts/check_status_contrast.py
	# The CONTRAST_CORE value test: pins the shared composited-contrast
	# arithmetic (wash-over-opaque, opacity-group) against hand-computed
	# numbers, via the same new Function serialization the gates use.
	# The 2026-10-22 instrument bug (a channel-alpha typo producing
	# negative luminances) passed every syntax check; only a VALUE test
	# catches that class.
	cd frontend && node scripts/test-contrast-core.mjs

# ── Visual + a11y audits (AGENTS.md §12) ───────────────────────
#
# Five gates over the RENDERED app, not the source:
#
#   probe.mjs         the static page probe - 27 routes x 3 widths x
#                     2 themes, with the overflow, contrast, tap-target,
#                     heading, font and type-scale checks
#   audit-targets.mjs the rendered TARGETS, driving the real app so the
#                     composer, its panels and the popovers are reached
#   audit-contrast.mjs the rendered CONTRAST, same driving, measuring
#                     every text element against its EFFECTIVE
#                     background
#   audit-analytics.mjs  the ANALYTICS dashboard with a seeded account:
#                        non-zero engagement, a DENSE posting-volume
#                        series, and a success rate that RECONCILES with
#                        the numbers printed beside it.
#   audit-dialogs.mjs   the DIALOG contract: a name, aria-modal, the
#                     focus inside it, and Escape that closes it (or
#                     asks first)
#   e2e-calendar-drag.mjs
#                     the calendar's DRAG-to-reschedule, end to end: a
#                     real drag, the wall clock converted in the app's
#                     zone, and the move sticking after a reload
#
# They need a RUNNING instance and the Chromium under
# ~/.cache/ms-playwright. Each exits 3 (BLOCKED) when the browser is
# missing - never "passed".
#
#   make audit URL=https://localhost:6544 PASSWORD=...
#
# The two browser-side audits are the ones that find what the static
# probe cannot: the probe only ever renders a page's DEFAULT state.
audit:
	@test -n "$(URL)" || (echo "URL is required" && exit 2)
	@test -n "$(PASSWORD)" || (echo "PASSWORD is required" && exit 2)
	cd frontend && node scripts/probe.mjs --url $(URL) --password '$(PASSWORD)' --out /tmp/sf-probe
	cd frontend && node scripts/audit-targets.mjs --url $(URL) --password '$(PASSWORD)' --out /tmp/sf-targets.json
	cd frontend && node scripts/audit-contrast.mjs --url $(URL) --password '$(PASSWORD)' --theme dark --out /tmp/sf-contrast-dark.json
	cd frontend && node scripts/audit-contrast.mjs --url $(URL) --password '$(PASSWORD)' --theme light --out /tmp/sf-contrast-light.json
	cd frontend && node scripts/audit-dialogs.mjs --url $(URL) --password '$(PASSWORD)'
	cd frontend && node scripts/audit-analytics.mjs --url $(URL) --password '$(PASSWORD)'
	cd frontend && node scripts/e2e-calendar-drag.mjs --url $(URL) --password '$(PASSWORD)'
	# The DEEP state walker: states none of the above reach (the media
	# panel, the comments thread's Hide, the channel/schedule/Tags panels
	# of the composer, search results, automation/webhook/signature
	# dialogs, post detail, kanban card). The 2026-10-22 red-team found
	# the 199x20 upload label and the 26x16 Hide exactly here - both
	# invisible to every default-state gate.
	cd frontend && node scripts/state-hunt.mjs --url $(URL) --password '$(PASSWORD)'
	# The calendar's non-default views, 9.50: each is its own component
	# with its own layout, and the default-view probe never renders them.
	# The 2026-10-20 red-team found the month view's chips at 15px while
	# every gate was green, because no audit pass drove the state.
	cd frontend && node scripts/probe.mjs --url $(URL) --password '$(PASSWORD)' --routes /calendar --state 'Month' --out /tmp/sf-probe-month
	cd frontend && node scripts/probe.mjs --url $(URL) --password '$(PASSWORD)' --routes /calendar --state 'List' --out /tmp/sf-probe-list
	cd frontend && node scripts/probe.mjs --url $(URL) --password '$(PASSWORD)' --routes /calendar --state 'Day' --out /tmp/sf-probe-day
	# The COMPOSER, open over the calendar (Phase 7 / M-1).
	#
	# This pass is why the 6 signature-overlap FAILs were found at all: the
	# composer is a state the main route probe never enters, because
	# /posts/new redirects to /posts and the dialog only stays open while
	# the founder is in it. AGENTS.md 9.50 - a page measured only in its
	# default state is a page whose other states are unchecked. The
	# state-hunt walker reaches the composer's PANELS; this reaches the
	# dialog itself, with the calendar behind it, which is what overlaps.
	cd frontend && node scripts/probe.mjs --url $(URL) --password '$(PASSWORD)' --routes /calendar --state 'New Post' --out /tmp/sf-probe-composer
	# Same route, second state: the preview tab open. Below lg the preview
	# is a separate pane the footer toggle reveals (M-1) - MEASURED: the
	# toggle changes the pane from display:none to block at 768 and 375,
	# and at 1440 the button is lg:hidden with the pane already visible,
	# so that width is a deliberate no-op rather than a surprise. The
	# phone widths are the ones this pass exists for.
	cd frontend && node scripts/probe.mjs --url $(URL) --password '$(PASSWORD)' --routes /calendar --state 'New Post,Preview' --out /tmp/sf-probe-composer-preview
	# The login page, in its OWN logged-out pass: the layout guard
	# redirects an authenticated visitor away from /login, so the main
	# probe can never measure it. This is the page that greets the
	# founder, and where the Unlock button's invisible text-white lived
	# hand-patched rather than gate-covered (the 9.45 finding).
	cd frontend && node scripts/probe.mjs --url $(URL) --password '$(PASSWORD)' --routes /login --logged-out --out /tmp/sf-probe-login
	# Each pass writes ~20MB of screenshots; the audit runs seven of them.
# This session hit EDQUOT on ~550MB of accumulated probe output and a
# later run FAILED on write errors that looked like app failures.
	rm -rf /tmp/sf-probe /tmp/sf-probe-month /tmp/sf-probe-list /tmp/sf-probe-day /tmp/sf-probe-login /tmp/sf-probe-composer /tmp/sf-probe-composer-preview

# ── Deploy / Redeploy ───────────────────────────────────────────

# Full deploy (build frontend + Rust, install skill, then restart)
deploy: build install-skill
	sudo install -m 755 $(RELEASE_DIR)/$(APP_NAME) $(BINARY_PATH)
	sudo systemctl daemon-reload
	sudo systemctl restart $(APP_NAME)
	@echo "✓ Deployed $(APP_NAME) (zigbuild, glibc 2.36)"

# One-step redeploy — the daily driver for active development.
# Skips frontend build for speed; use `make deploy` when frontend changes.
redeploy: install-skill
	cargo zigbuild --release --target $(TARGET) && sudo install -m 755 $(RELEASE_DIR)/$(APP_NAME) $(BINARY_PATH) && sudo systemctl restart $(APP_NAME)
	@echo "✓ Redeployed $(APP_NAME) (zigbuild, glibc 2.36)"

# ── Service Management ──────────────────────────────────────────

restart:
	sudo systemctl daemon-reload
	sudo systemctl restart $(APP_NAME)

status:
	@echo "=== systemd ==="
	systemctl status $(APP_NAME) --no-pager || true
	@echo ""
	@echo "=== database ==="
	@echo "$(APP_DIR)/data/switchpost.db"

logs:
	journalctl -u $(APP_NAME) -n 50 --no-pager -f

# ── Install AI Agent Skill ──────────────────────────────────────
# Copy every reference, not a hardcoded list: adding a reference doc without
# touching this target used to leave it silently uninstalled, and the docs that
# point at it (SKILL.md, PAGE-INDEX) then referenced a file the agent could not read.
install-skill:
	@mkdir -p $(SKILL_DEST)/references
	@cp $(SKILL_SRC)/SKILL.md $(SKILL_DEST)/SKILL.md
	@cp $(SKILL_SRC)/references/*.md $(SKILL_DEST)/references/
	@# Remove leaves the repo no longer ships: a renamed or deleted reference
	@# used to linger in the install dir, and an agent reading it got stale
	@# commands the doc-CLI guard had never seen.
	@for f in $(SKILL_DEST)/references/*.md; do \
	   b=$$(basename $$f); \
	   [ -f "$(SKILL_SRC)/references/$$b" ] || { echo "  → removing stale $$b"; rm "$$f"; }; \
	 done
	@for f in $(SKILL_SRC)/references/*.md; do \
	   printf '  → %s\n' "$$(basename $$f)"; done
	@echo "✓ Installed skill to $(SKILL_DEST)"

# ── Auto-watch (requires cargo-watch) ──────────────────────────
#   cargo install cargo-watch
watch:
	cargo watch -x 'zigbuild --release --target $(TARGET)' -s 'sudo install -m 755 $(RELEASE_DIR)/$(APP_NAME) $(BINARY_PATH) && sudo systemctl restart $(APP_NAME)'
