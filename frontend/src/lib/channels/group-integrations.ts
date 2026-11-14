import type { Integration } from "$lib/api/integrations";

/**
 * Group connected channels by the heading they belong under.
 *
 * ── THE DEFECT THIS CLOSES ─────────────────────────────────────
 * The old grouping keyed on `provider_name`, and two providers with the
 * same account behind them report DIFFERENT names, so one login produced
 * several channel-list headings. Keying on the provider identifier keeps
 * two integrations of the same provider under one heading without
 * relying on the display name being identical.
 *
 * ── WHY IT IS ONE HEADING PER PROVIDER ─────────────────────────
 * The Google family used to collapse into a single "Google Suite"
 * heading. That was a promise the unified `google` provider kept - and
 * it was the wrong promise: the founder's six YouTube channels showed
 * under a heading that named none of them ("Google Suite (7)"). The
 * unified provider is gone (it published YouTube videos and nothing
 * else), so each Google service is its own provider with its own consent
 * and its own heading.
 */
export function groupIntegrations(
  integrations: Integration[]
): Map<string, Integration[]> {
  return integrations.reduce((g, int) => {
    // One heading per PROVIDER.
    //
    // The Google family used to collapse into a single "Google Suite"
    // heading, which read as one account - and showed the founder's six
    // YouTube channels under a heading that named none of them ("Google
    // Suite (7)"). Each Google service is its own consent and its own
    // provider now, so each keeps its own heading.
    const key = int.provider_identifier;

    // A Map's key is not reactive state in Svelte 5 (§9.22), so the caller
    // reads a fresh Map each time `integrations` changes. That is why this
    // builds and returns a new Map rather than being handed one to mutate.
    const existing = g.get(key) || [];
    existing.push(int);
    return g.set(key, existing);
  }, new Map<string, Integration[]>());
}

/**
 * The heading a group's integrations should display.
 *
 * The provider's own display name, so the list never renames something the
 * founder connected under a different name.
 */
export function groupHeading(
  key: string,
  integrations: Integration[]
): string {
  return integrations[0]?.provider_name || key;
}
