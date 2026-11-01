// v25 F4 - ChannelCard status clarity.
//
// v21 shipped a 2px colored dot with a `title` attribute as the only signal
// for "is this channel healthy?". A `title` is invisible until hover, is
// unavailable to touch and to screen readers in practice, and cannot express
// *why* a channel is unhealthy. Every other surface in the app already had a
// labeled status chip; the channel list was the outlier.
//
// This module is the single source of truth for that signal. It is a pure
// function so the card, the context menu, and any future status filter all
// agree on what "expiring" means - the bug class where two components each
// re-derive a status slightly differently is what made this unreadable.
//
// Precedence is deliberate and total: a disabled channel that also has a
// dead token is reported as *disabled*, because "turn it back on" is the
// action the user can actually take. Surfacing "token dead" on a channel the
// user deliberately switched off trains them to ignore the warning.

import type { Integration } from "$lib/api/integrations";

export type ChannelState = "connected" | "expiring" | "refresh_needed" | "disabled";

export interface ChannelStatus {
  state: ChannelState;
  /** Short chip label. Must read as a word, not a color. */
  label: string;
  /** One line explaining the state and the action that resolves it. */
  hint: string;
  /** Icon key from $lib/ui/Icon.svelte. */
  icon: string;
  /** Tailwind classes built ONLY from F1 token utilities. */
  chip: string;
  /** Just the text tier, for the icon-only variant below `sm`. */
  textClass: string;
  /** Whether the card should offer a "refresh token" / "reconnect" action. */
  actionable: boolean;
}

/** A token inside this window is "expiring" rather than "connected". */
const EXPIRING_WINDOW_MS = 7 * 24 * 60 * 60 * 1000;

/**
 * Resolve the connection state of an integration.
 *
 * `token_expires_at` is optional: `IntegrationPublic` in src/db/models.rs
 * does not expose it yet, so the API never sends it today. It is declared
 * anyway so the UI is correct the moment the field is added server-side
 * rather than needing a second pass then. Everything else keys off fields
 * that ARE returned today.
 */
export function channelStatus(integration: Integration, now: Date = new Date()): ChannelStatus {
  if (integration.disabled) {
    return {
      state: "disabled",
      label: "Disabled",
      hint: "Not posting. Toggle it back on to resume.",
      icon: "close",
      chip: "text-muted border-line bg-surface-hover",
      textClass: "text-muted",
      actionable: false,
    };
  }

  if (integration.refresh_needed) {
    return {
      state: "refresh_needed",
      label: "Needs reconnect",
      // DO NOT assert a cause here. This hint used to blame the refresh
      // itself - a claim the client cannot make.
      // `IntegrationPublic` (src/db/models.rs) carries only the boolean
      // `refresh_needed`, with no failure reason, so the cause is NOT in the
      // payload this function is given.
      //
      // Production proved the old wording was actively wrong. The backend
      // (src/feed/mod.rs, `classify_feed_failure`) now distinguishes a token
      // problem from a gone target, and for these channels it logs "The
      // channel's token is fine, so it stays publishable; the target is
      // gone" while the row still carries the stale pre-fix flag. So the
      // founder was told the token failed, saw it reconnect, and watched the
      // channel fail again - because the page, not the token, was the problem.
      //
      // Reconnecting is still the right action to OFFER, so `actionable` stays
      // true; only the false diagnosis is removed.
      //
      // NOTE the phrasing here deliberately avoids the two literal sentences
      // this file used to carry, because tests/channels_grid_multi_account_test.rs
      // asserts their ABSENCE over this source - a comment that repeats a
      // condemned phrase would fail its own guard.
      hint: "This channel needs attention. Reconnect it from Channels, then check its posts still load.",
      icon: "refresh",
      chip: "text-content border-error/50 bg-error/10",
      textClass: "text-error",
      actionable: true,
    };
  }

  const expiry = integration.token_expires_at ? Date.parse(integration.token_expires_at) : NaN;
  if (Number.isFinite(expiry)) {
    const msLeft = expiry - now.getTime();
    if (msLeft <= 0) {
      return {
        state: "refresh_needed",
        label: "Token expired",
        hint: "The access token is past its expiry. Refresh it to resume posting.",
        icon: "refresh",
        chip: "text-content border-error/50 bg-error/10",
        textClass: "text-error",
        actionable: true,
      };
    }
    if (msLeft <= EXPIRING_WINDOW_MS) {
      const days = Math.max(1, Math.round(msLeft / 86_400_000));
      return {
        state: "expiring",
        label: "Expiring soon",
        hint: `Token expires in ${days} day${days === 1 ? "" : "s"}. Refresh it to avoid a posting gap.`,
        icon: "clock",
        chip: "text-content border-warning/50 bg-warning/10",
        textClass: "text-warning",
        actionable: true,
      };
    }
  }

  return {
    state: "connected",
    label: "Connected",
    hint: "Healthy. Scheduled posts will publish.",
    icon: "check",
    chip: "text-content border-success/50 bg-success/10",
    textClass: "text-success",
    actionable: false,
  };
}

/** True when a channel needs the user's attention to keep publishing. */
export function needsAttention(integration: Integration): boolean {
  return channelStatus(integration).actionable;
}
