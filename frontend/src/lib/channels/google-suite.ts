/**
 * Google Suite consent grouping.
 *
 * ── WHAT THIS IS ────────────────────────────────────────────────
 *
 * Google's authorize endpoint REFUSES two of the scope pairs this app can
 * request, and it refuses them BEFORE any consent screen is shown. Verified
 * live 2026-10-30 against the deployed instance:
 *
 *   - a YouTube Data API scope beside a Drive scope → `Error 400: invalid_request`
 *   - YouTube beside `calendar.events`                → refused the same way
 *
 * The rule is pinned server-side in `src/social/scope_guard.rs`
 * (`GOOGLE_CONFLICTING_SCOPE_FAMILIES`) and enforced by a test in
 * `tests/connect_flow_redteam_test.rs`. It is by FAMILY rather than by exact
 * scope string, so a scope added later (`youtube.readonly`) is caught without
 * editing the table.
 *
 * So the operator's ask - one login flow covering Google, Gmail, Calendar,
 * Drive and YouTube - cannot be satisfied as stated, and the reason is
 * measured rather than guessed: the BACKEND already narrowed the `google`
 * provider's scope set to escape the live 400, and that narrowing means
 * `drive` is necessarily its own consent and `youtube` its own tile.
 * What CAN be built is one consent for the three providers whose scopes
 * coexist, which is the closest reachable shape of the ask.
 *
 * ── THE BACKEND IS ALREADY UNIFIED ──────────────────────────────
 *
 * This is not a backend change, and it must not become one. All six
 * Google-family providers already share ONE credential pair and ONE OAuth
 * implementation:
 *
 *   - `Config::provider_credentials` (src/config.rs:441-445) resolves
 *     `google`, `google_my_business`, `gmail`, `calendar` AND `drive` to the
 *     same `YOUTUBE_CLIENT_ID` / `YOUTUBE_CLIENT_SECRET`, in one match arm.
 *   - `src/social/google_oauth.rs` is the single `exchange_code` /
 *     `refresh` / `profile_picture` for google, gmail, calendar, drive,
 *     google_my_business and youtube.
 *
 * The CLIENT_ID/SECRET integration is therefore already effective at the
 * backend. What was missing is that the FRONTEND offered five separate
 * tiles for one credential pair, which reads as "five different logins" to
 * the founder and made the unified credential look like a bug.
 *
 * ── THE RULE ────────────────────────────────────────────────────
 *
 * Group A (one consent, one tile): google, gmail, calendar.
 * Group B (its own consent, its own tile): youtube, drive, google_my_business.
 *
 * A group tile opens the SAME `ConnectFlow` the individual providers open,
 * so no provider becomes unreachable - the group is a presentation of the
 * union, not a replacement, and a member that is not in the group still has
 * its own tile on the grid.
 */

/** The providers whose Google scopes coexist in ONE consent request.
 *
 * THIS LIST WAS WRONG TWICE and the test caught it both times, which is the
 * reason the mechanical check below exists.
 *
 * Draft 1 was google+gmail+calendar+drive. That union is REFUSED by Google
 * before any consent screen: `google` carries YouTube scopes, `drive` carries
 * Drive scopes, and that pair is the one Google names in the live 2026-10-30
 * `invalid_request`.
 *
 * Draft 2 was google+gmail+calendar. Also refused, for a subtler reason: the
 * `calendar` provider requests BOTH `calendar.readonly` and `calendar.events`,
 * and a YouTube scope beside `calendar.events` is Google's SECOND conflicting
 * pair (`GOOGLE_CONFLICTING_SCOPE_FAMILIES`). The `google` provider's own
 * comment explains why it asks for the bare `calendar` scope instead -
 * requesting `calendar.events` alongside is how the whole class of failure
 * starts. `calendar` as a provider cannot ride the same consent.
 *
 * MEASURED SURVIVOR: `google` + `gmail`. `gmail`'s full scope set
 * (gmail.readonly, gmail.send, gmail.labels, gmail.modify) does not collide
 * with google's YouTube + calendar grant, so both providers' needs are
 * satisfiable from one consent. Everything else in the Google family is
 * necessarily its own consent.
 *
 * So the tile is honest about what it covers: Gmail rides on the `google`
 * consent; Calendar and Drive each have their own connect.
 */
export const GOOGLE_SUITE_GROUP: readonly string[] = [
  'google',
  'gmail',
] as const;

/**
 * Google providers that CANNOT join the suite group, each for a measured
 * reason rather than a preference:
 *
 *   - `calendar`: it requests `calendar.events`, which conflicts with the
 *     YouTube scope `google`'s consent carries. Its own connect asks only for
 *     calendar scopes, which Google grants on its own.
 *   - `drive`: a Drive scope beside a YouTube scope is the pair Google names
 *     in the live `invalid_request`. Since `google` grants YouTube, Drive
 *     cannot be in the same consent.
 *   - `youtube`: its scope set beside Drive's is the other refused pair, so a
 *     standalone YouTube connect stays standalone. Note the `google` consent
 *     already grants YouTube, so this tile is the one to use for a
 *     YouTube-ONLY connect.
 *   - `google_my_business`: a different product surface with its own scopes;
 *     the frontend labels it "Google Biz" and it keeps its own tile.
 */
export const GOOGLE_SEPARATE_PROVIDERS: readonly string[] = [
  'calendar',
  'drive',
  'youtube',
  'google_my_business',
] as const;

/** Everything Google-family that the app offers. */
export const GOOGLE_FAMILY_PROVIDERS: readonly string[] = [
  ...GOOGLE_SUITE_GROUP,
  ...GOOGLE_SEPARATE_PROVIDERS,
] as const;

/** The label the suite group's tile carries. */
export const GOOGLE_SUITE_LABEL = 'Google Suite';

/**
 * A one-sentence reason the group exists, shown under the tile.
 *
 * Carried in code rather than in a comment-only form so the UI, the docs and
 * a test all read the SAME sentence. The founder has to be told why YouTube
 * is not in here - otherwise the tile looks broken.
 */
export const GOOGLE_SUITE_NOTE =
  'One tile for the whole Google account. Google refuses some scope pairs together, so Calendar, Drive, YouTube and Google Biz each run their own consent - pick one below.';

/**
 * The label each suite choice shows in the chooser, so the founder sees the
 * service name rather than the internal provider id.
 *
 * Kept here rather than in the page so the label and the provider id cannot
 * drift apart, and so a test can assert every choice has a label.
 */
export const GOOGLE_SUITE_CHOICE_LABELS: Record<string, string> = {
  gmail: 'Gmail',
  calendar: 'Calendar',
  drive: 'Drive',
  youtube: 'YouTube',
  google_my_business: 'Google Business Profile',
};

/** Is this provider one of the suite group's members? */
export function isGoogleSuiteMember(provider: string): boolean {
  return (GOOGLE_SUITE_GROUP as readonly string[]).includes(provider);
}

/** Is this provider Google-family at all (suite, separate, or the GMB variant)? */
export function isGoogleFamily(provider: string): boolean {
  return (GOOGLE_FAMILY_PROVIDERS as readonly string[]).includes(provider);
}

/**
 * The provider a group tile opens `ConnectFlow` with.
 *
 * `google` is the group's representative because its consent screen is the
 * one the operator already recognises (it is the "Google Suite" card in the
 * Google Cloud console), and because its scope set is the broadest that
 * Google will still grant in one request - YouTube, Gmail read/send and
 * Calendar all ride on that one consent. The founder then enables the
 * services they want on Google's own consent screen.
 */
export const GOOGLE_SUITE_REPRESENTATIVE = 'google';

/**
 * Expand a provider list into what the connect grid should render.
 *
 * ── THE REDUNDANCY THIS REMOVES ────────────────────────────────
 * The grid used to show SIX Google-family tiles (Google, Gmail,
 * Calendar, Drive, YouTube and Google Biz). All six resolve to the SAME
 * client id/secret in `Config::provider_credentials` and the SAME
 * `social/google_oauth.rs`; the difference between them is only which
 * SCOPES that one credential asks for. Offering six tiles for one
 * credential reads to the founder as six separate logins - the redundancy
 * this collapses.
 *
 * ── WHAT THE COLLAPSE DOES NOT DO ──────────────────────────────
 * It does NOT merge the consents. Google refuses certain scope pairs
 * (`GOOGLE_CONFLICTING_SCOPE_FAMILIES`), so Calendar, Drive and YouTube
 * still run their own OAuth round. The tile opens a chooser for the
 * family, so every provider stays reachable with one click - the group is
 * a PRESENTATION of the union, not a replacement.
 *
 * Exported so a test can assert the shape without rendering the page: the
 * grid is a pure function of the provider list, and the pure function is
 * the thing worth pinning.
 */
export function collapseGoogleSuite(
  providers: readonly string[],
): { standalone: string[]; suiteMemberCount: number } {
  const standalone: string[] = [];
  let suiteMemberCount = 0;
  for (const p of providers) {
    if (isGoogleFamily(p)) {
      suiteMemberCount += 1;
      // The one suite tile stands in at the FIRST Google-family member's
      // position, so the grid does not reshuffle when the grouping lands.
      if (suiteMemberCount === 1) standalone.push(GOOGLE_SUITE_REPRESENTATIVE);
      continue;
    }
    standalone.push(p);
  }
  return { standalone, suiteMemberCount };
}

/**
 * The Google-family providers the suite tile offers, in the order the
 * chooser shows them.
 *
 * Derived from the family list rather than hand-written, so a provider
 * added to `GOOGLE_FAMILY_PROVIDERS` appears here automatically. This is
 * the list that keeps every member REACHABLE after the grid collapses
 * them into one tile.
 */
export function googleSuiteChoices(): readonly string[] {
  return GOOGLE_FAMILY_PROVIDERS.filter((p) => p !== GOOGLE_SUITE_REPRESENTATIVE);
}
