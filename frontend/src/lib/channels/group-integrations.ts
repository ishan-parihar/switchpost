import type { Integration } from "$lib/api/integrations";
import { isGoogleFamily } from "./google-suite";

/**
 * Group connected channels by the heading they belong under.
 *
 * ── THE DEFECT THIS CLOSES ─────────────────────────────────────
 * The old grouping keyed on `provider_name`, and the Google-family names
 * are all DIFFERENT: `google` reports "Google Suite", `gmail` reports
 * "Gmail", `calendar` reports "Google Calendar", `drive` reports
 * "Google Drive", `youtube` reports "YouTube".
 *
 * One "Connect Google" on the connect grid therefore produced three
 * channel-list headings —
 *
 *     Google Suite (1)
 *     Gmail (1)
 *     Google Calendar (1)
 *
 * — for a single login, on the same account, which is the redundancy the
 * founder reported: the unified Google-Suite option is not unified once
 * the channels land in the list. The count was also always 1, so the
 * heading could never say how many Google accounts there really were.
 *
 * THE RULE: the Google family is ONE credential pair (`provider_credentials`
 * resolves every one of them to the same client id/secret, and
 * `social/google_oauth.rs` is the single exchange/refresh), so the list
 * must present it as one group. Every other provider keeps its own
 * heading, unchanged.
 *
 * The key is the PROVIDER IDENTIFIER, not the display name: two
 * integrations of the same provider share a heading without relying on
 * the display name being identical.
 */
export function groupIntegrations(
  integrations: Integration[]
): Map<string, Integration[]> {
  return integrations.reduce((g, int) => {
    // One heading for the whole Google family, so a founder who connected
    // Google once sees ONE group with their accounts under it.
    const key = isGoogleFamily(int.provider_identifier)
      ? "Google Suite"
      : int.provider_identifier;

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
 * A Google-family group is shown as "Google Suite"; every other group keeps
 * the provider's own display name, so the list never renames something the
 * founder connected under a different name.
 */
export function groupHeading(
  key: string,
  integrations: Integration[]
): string {
  if (key === "Google Suite") return "Google Suite";
  return integrations[0]?.provider_name || key;
}
