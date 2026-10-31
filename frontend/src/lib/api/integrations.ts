import { api } from './client';

export interface Integration {
  id: string; provider_identifier: string; provider_name: string;
  internal_id: string;
  profile_name?: string; profile_picture?: string; profile_url?: string;
  disabled: boolean; refresh_needed: boolean;
  posting_times?: { time: number }[];
  auth_method?: string;
  root_internal_id?: string;
  /**
   * v25 F4: ISO-8601 expiry of the stored access token, when the API sends
   * one. Optional on purpose - `IntegrationPublic` in src/db/models.rs does
   * NOT expose `token_expires_at` yet, so today this is always undefined and
   * channelStatus() falls back to connected / refresh-needed. It is declared
   * ahead of the backend so enabling the "expiring" status is a one-line Rust
   * change rather than a second frontend pass. See
   * lib/channels/channel-status.ts.
   */
  token_expires_at?: string | null;
}

export interface TimeslotEntry {
  time: number; // minutes from midnight
}

export interface ConnectApiKeyRequest {
  provider: string;
  api_key: string;
  instance_url?: string;
  label?: string;
  verification_code?: string;
}

export interface ConnectWeb3Request {
  provider: string;
  address: string;
  label?: string;
}

export interface PageInfo {
  id: string; name: string; access_token?: string;
  picture?: string; username?: string;
}

/**
 * One provider as the server reports it.
 *
 * `char_limit` is the provider's own content limit, read from the
 * provider implementation - the same number the server validates against.
 * The frontend used to hardcode these in a TypeScript table and the two
 * drifted.
 */
export interface ProviderStatus {
  identifier: string;
  name: string;
  configured: boolean;
  oauth: boolean;
  has_credentials: boolean;
  editor_type: string;
  redirect_uri: string;
  tier: string;
  char_limit: number;
  /**
   * Whether the PROVIDER has a direct-message surface at all - read from
   * the provider implementation, not guessed here.
   *
   * The /dms page used to carry its own hardcoded
   * `['x','instagram','linkedin']` and pick the first match as the default
   * channel. On an instance whose only X channel is COOKIE-auth that landed
   * on a 400, because X's DM API is bearer-only. A client-side guess
   * (`auth_method === 'oauth'`) just moved the drift and still offered X in
   * the same selector.
   *
   * NOTE this is the PROVIDER's capability, not a given CHANNEL's. An X
   * channel authenticated with cookies still cannot reach X's DMs - that
   * is the channel-level check the page also has to make.
   */
  dm_supported: boolean;
}

export interface TargetInfo {
  id: string;
  name: string;
  target_type: string;
  picture?: string;
  metadata?: Record<string, unknown>;
}

export const integrationsApi = {
  list: () => api.get<{ integrations: Integration[] }>("/api/integrations"),
  /** Every registered provider, with the content limit the server enforces. */
  listProviders: () => api.get<ProviderStatus[]>("/api/providers"),
  connect: (provider: string) => api.get<{ url: string; state: string }>(`/api/integrations/connect/${provider}`),
  connectApiKey: (body: ConnectApiKeyRequest) =>
    api.post<{ integration: Integration }>("/api/integrations/connect/api-key", body),
  connectWeb3: (body: ConnectWeb3Request) =>
    api.post<{ integration: Integration }>("/api/integrations/connect/web3", body),
  disconnect: (id: string) => api.del<{ deleted: boolean }>(`/api/integrations/${id}`),
  updateTimeslots: (id: string, timeslots: TimeslotEntry[]) =>
    api.put<{ success: boolean; posting_times: TimeslotEntry[] }>(
      `/api/integrations/${id}/timeslots`,
      { timeslots }
    ),
  toggleDisable: (id: string, disabled: boolean) =>
    api.put<{ success: boolean; disabled: boolean }>(
      `/api/integrations/${id}/disable`,
      { disabled }
    ),
  refresh: (id: string) =>
    api.post<{ success: boolean }>(`/api/integrations/${id}/refresh`),
  availablePages: (integrationId: string) =>
    api.get<{ pages: PageInfo[]; parent_integration_id: string; provider: string }>(
      `/api/integrations/${integrationId}/available-pages`
    ),
  connectPage: (parentIntegrationId: string, pageId: string) =>
    api.post<{ integration: Integration; parent_id: string }>(
      `/api/integrations/${parentIntegrationId}/connect-page/${pageId}`
    ),
  connectXCookie: (auth_token: string, ct0: string) =>
    api.post<{ integration: Integration }>("/api/integrations/connect/x-cookie", { auth_token, ct0 }),
  connectRedditCookie: (cookie_string: string) =>
    api.post<{ integration: { id: string; provider_name: string; profile_name: string } }>("/api/integrations/connect/reddit-cookie", { cookie_string }),
  connectGithubPat: (pat: string, label?: string) =>
    api.post<{ integration: Integration }>("/api/integrations/connect/github-pat", { pat, label }),
  verifyOneTimeToken: (provider: string, code: string) =>
    api.post<{ url: string; state: string }>(`/api/integrations/connect/${provider}/verify`, { code }),
  whatsappPair: (phone_number: string) =>
    api.post<{ pair_code: string; expires_in: number }>("/api/integrations/connect/whatsapp/pair", { phone_number }),
  whatsappStatus: () =>
    api.get<{ authenticated: boolean; jid?: string }>("/api/integrations/connect/whatsapp/status"),
  telegramUserRequestCode: (phone: string) =>
    api.post<{ status: string }>("/api/integrations/connect/telegram-user/request-code", { phone }),
  telegramUserSignIn: (code: string) =>
    api.post<{ integration: Integration }>("/api/integrations/connect/telegram-user/sign-in", { code }),
  connectTelegramBotToken: (token: string) =>
    api.post<{ integration: Integration }>("/api/integrations/connect/telegram-bot/token", { token }),
  listTargets: (integrationId: string) =>
    api.get<{ targets: TargetInfo[]; integration_id: string; provider: string }>(
      `/api/integrations/${integrationId}/targets`
    ),
  /** v26-5: Search for users/accounts to @mention on a platform. */
  searchMentions: (integrationId: string, q: string) =>
    api.get<{ results: Array<{ id: string; label: string; formatted: string; image?: string | null; provider: string }> }>(
      `/api/integrations/${integrationId}/mentions?q=${encodeURIComponent(q)}`
    ),
};
